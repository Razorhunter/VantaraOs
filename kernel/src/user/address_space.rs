use crate::sync::PreemptMutex as Mutex;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::tlb;
use x86_64::{
    PhysAddr, VirtAddr,
    registers::control::Cr3,
    structures::paging::{
        FrameAllocator, Page, PageSize, PageTable, PageTableFlags, PhysFrame, Size4KiB,
        page_table::FrameError,
    },
};

const PRIVATE_P4_POOL_SIZE: usize = 8;
const MAX_PRIVATE_PROGRAM_PAGE_COUNT: usize = 64;
const PRIVATE_STACK_SLOT_COUNT: usize = crate::user::ring3::USER_STACK_SLOT_COUNT as usize;
const PRIVATE_STACK_PAGE_COUNT: usize = crate::user::ring3::USER_STACK_PAGE_COUNT as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceKind {
    Kernel,
    KernelSharedUser,
    PreparedUser,
    IsolatedUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressSpace {
    pub kind: AddressSpaceKind,
    pub p4_frame: Option<u64>,
    pub p4_verified: bool,
    pub entry_point: u64,
    pub layout: crate::user::ring3::UserMemoryLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceError {
    FrameAllocationFailed,
    MissingPhysicalMemoryOffset,
    NoKernelP4,
    NoPreparedP4,
    PrivateMappingNotFound,
    PrivateMappingTooSmall,
    SwitchVerificationFailed,
    UserMappingNotAccessible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserPageAccess {
    Read,
    Write,
}

impl AddressSpace {
    pub const fn kernel_placeholder(entry_point: u64) -> Self {
        Self {
            kind: AddressSpaceKind::Kernel,
            p4_frame: None,
            p4_verified: false,
            entry_point,
            layout: crate::user::ring3::UserMemoryLayout::kernel_placeholder(),
        }
    }

    pub const fn kernel_shared_user(
        entry_point: u64,
        layout: crate::user::ring3::UserMemoryLayout,
    ) -> Self {
        Self {
            kind: AddressSpaceKind::KernelSharedUser,
            p4_frame: None,
            p4_verified: false,
            entry_point,
            layout,
        }
    }

    pub const fn isolated_user(
        entry_point: u64,
        layout: crate::user::ring3::UserMemoryLayout,
        p4_frame: u64,
    ) -> Self {
        Self {
            kind: AddressSpaceKind::IsolatedUser,
            p4_frame: Some(p4_frame),
            p4_verified: true,
            entry_point,
            layout,
        }
    }

    pub const fn prepared_user(
        entry_point: u64,
        layout: crate::user::ring3::UserMemoryLayout,
        p4_frame: u64,
    ) -> Self {
        Self {
            kind: AddressSpaceKind::PreparedUser,
            p4_frame: Some(p4_frame),
            p4_verified: false,
            entry_point,
            layout,
        }
    }

    pub const fn verified_prepared_user(
        entry_point: u64,
        layout: crate::user::ring3::UserMemoryLayout,
        p4_frame: u64,
    ) -> Self {
        Self {
            kind: AddressSpaceKind::PreparedUser,
            p4_frame: Some(p4_frame),
            p4_verified: true,
            entry_point,
            layout,
        }
    }

    pub const fn promoted_isolated_user(self) -> Self {
        match self.p4_frame {
            Some(p4_frame) if self.p4_verified => {
                Self::isolated_user(self.entry_point, self.layout, p4_frame)
            }
            _ => self,
        }
    }

    pub const fn user_stack_top(&self) -> u64 {
        self.layout.stack_top
    }

    pub const fn is_isolated(&self) -> bool {
        matches!(self.kind, AddressSpaceKind::IsolatedUser)
    }

    pub const fn has_verified_p4(&self) -> bool {
        self.p4_verified
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreparedP4Slot {
    p4_frame: u64,
    program_frames: [u64; MAX_PRIVATE_PROGRAM_PAGE_COUNT],
    program_frame_count: usize,
    stack_frames: [[u64; PRIVATE_STACK_PAGE_COUNT]; PRIVATE_STACK_SLOT_COUNT],
    stack_in_use: [bool; PRIVATE_STACK_SLOT_COUNT],
    in_use: bool,
}

impl PreparedP4Slot {
    const EMPTY: Self = Self {
        p4_frame: 0,
        program_frames: [0; MAX_PRIVATE_PROGRAM_PAGE_COUNT],
        program_frame_count: 0,
        stack_frames: [[0; PRIVATE_STACK_PAGE_COUNT]; PRIVATE_STACK_SLOT_COUNT],
        stack_in_use: [false; PRIVATE_STACK_SLOT_COUNT],
        in_use: false,
    };
}

struct PrivateP4Pool {
    slots: [PreparedP4Slot; PRIVATE_P4_POOL_SIZE],
    count: usize,
}

impl PrivateP4Pool {
    const fn new() -> Self {
        Self {
            slots: [PreparedP4Slot::EMPTY; PRIVATE_P4_POOL_SIZE],
            count: 0,
        }
    }

    fn push(&mut self, slot: PreparedP4Slot) -> bool {
        if self.count >= self.slots.len() {
            return false;
        }
        self.slots[self.count] = slot;
        self.count += 1;
        true
    }

    fn take(&mut self) -> Option<u64> {
        let slot = self.slots[..self.count]
            .iter_mut()
            .find(|slot| !slot.in_use)?;
        slot.in_use = true;
        Some(slot.p4_frame)
    }

    fn peek_next(&self) -> Option<u64> {
        self.slots[..self.count]
            .iter()
            .find(|slot| !slot.in_use)
            .map(|slot| slot.p4_frame)
    }

    fn find_index(&self, p4_frame: u64) -> Option<usize> {
        self.slots[..self.count]
            .iter()
            .position(|slot| slot.p4_frame == p4_frame)
    }

    fn release(&mut self, p4_frame: u64) -> bool {
        let Some(slot) = self.slots[..self.count]
            .iter_mut()
            .find(|slot| slot.p4_frame == p4_frame)
        else {
            return false;
        };
        slot.in_use = false;
        slot.stack_in_use.fill(false);
        true
    }

    fn reserve_stack_slot(&mut self, p4_frame: u64, main_stack_top: u64) -> Option<(u64, u64)> {
        let slot = self.slots[..self.count]
            .iter_mut()
            .find(|slot| slot.p4_frame == p4_frame && slot.in_use)?;
        let main_index = stack_slot_index(main_stack_top)?;
        slot.stack_in_use[main_index] = true;
        let index = slot.stack_in_use.iter().position(|in_use| !*in_use)?;
        slot.stack_in_use[index] = true;
        Some(stack_range_for_slot(index))
    }

    fn release_stack_slot(&mut self, p4_frame: u64, stack_top: u64) -> bool {
        let Some(slot) = self.slots[..self.count]
            .iter_mut()
            .find(|slot| slot.p4_frame == p4_frame && slot.in_use)
        else {
            return false;
        };
        let Some(index) = stack_slot_index(stack_top) else {
            return false;
        };
        slot.stack_in_use[index] = false;
        true
    }
}

static PRIVATE_P4_POOL: Mutex<PrivateP4Pool> = Mutex::new(PrivateP4Pool::new());
static KERNEL_P4_FRAME: Mutex<Option<u64>> = Mutex::new(None);
static PHYSICAL_MEMORY_OFFSET: Mutex<Option<u64>> = Mutex::new(None);
static PHYSICAL_MEMORY_OFFSET_VALUE: AtomicU64 = AtomicU64::new(0);

pub fn init_private_p4_pool(
    physical_memory_offset: VirtAddr,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> usize {
    record_kernel_p4_frame();
    let offset = physical_memory_offset.as_u64();
    PHYSICAL_MEMORY_OFFSET_VALUE.store(offset, Ordering::Relaxed);
    *PHYSICAL_MEMORY_OFFSET.lock() = Some(offset);
    let program_page_count = required_private_program_pages();
    crate::serial_println!(
        "[USER] private P4 slots will reserve {} program page(s) each",
        program_page_count
    );

    let mut pool = PRIVATE_P4_POOL.lock();
    while pool.count < PRIVATE_P4_POOL_SIZE {
        let Some(frame) = frame_allocator.allocate_frame() else {
            break;
        };

        let Ok(slot) = (unsafe {
            prepare_private_p4_slot(
                physical_memory_offset,
                frame,
                program_page_count,
                frame_allocator,
            )
        }) else {
            break;
        };
        let _ = pool.push(slot);
    }

    pool.count
}

pub fn take_prepared_p4_frame() -> Option<u64> {
    PRIVATE_P4_POOL.lock().take()
}

pub fn release_prepared_p4_frame(p4_frame: u64) -> bool {
    let released = PRIVATE_P4_POOL.lock().release(p4_frame);
    if released {
        crate::serial_println!("[USER] released private P4 frame {:#x}", p4_frame);
    }
    released
}

pub fn reserve_private_thread_stack(p4_frame: u64, main_stack_top: u64) -> Option<(u64, u64)> {
    let range = PRIVATE_P4_POOL
        .lock()
        .reserve_stack_slot(p4_frame, main_stack_top)?;
    if clear_private_thread_stack(p4_frame, range.1).is_err() {
        let _ = release_private_thread_stack(p4_frame, range.1);
        return None;
    }
    Some(range)
}

pub fn release_private_thread_stack(p4_frame: u64, stack_top: u64) -> bool {
    PRIVATE_P4_POOL
        .lock()
        .release_stack_slot(p4_frame, stack_top)
}

fn clear_private_thread_stack(p4_frame: u64, stack_top: u64) -> Result<(), AddressSpaceError> {
    let physical_memory_offset = physical_memory_offset()?;
    let pool = PRIVATE_P4_POOL.lock();
    let slot = pool.slots[..pool.count]
        .iter()
        .find(|slot| slot.p4_frame == p4_frame && slot.in_use)
        .ok_or(AddressSpaceError::PrivateMappingNotFound)?;
    let stack_slot =
        stack_slot_index(stack_top).ok_or(AddressSpaceError::PrivateMappingNotFound)?;
    for frame in slot.stack_frames[stack_slot] {
        let destination = (physical_memory_offset + frame).as_mut_ptr::<u8>();
        // SAFETY: every address comes from a live private stack frame owned by
        // this prepared P4 slot, and the boot physical-memory mapping spans
        // the complete 4 KiB frame.
        unsafe {
            core::ptr::write_bytes(destination, 0, Size4KiB::SIZE as usize);
        }
    }
    Ok(())
}

fn stack_range_for_slot(index: usize) -> (u64, u64) {
    let stack_top = crate::user::ring3::FIRST_USER_STACK_TOP
        - index as u64 * crate::user::ring3::USER_STACK_SLOT_SIZE;
    (stack_top - crate::user::ring3::USER_STACK_SIZE, stack_top)
}

fn stack_slot_index(stack_top: u64) -> Option<usize> {
    if stack_top > crate::user::ring3::FIRST_USER_STACK_TOP {
        return None;
    }
    let offset = crate::user::ring3::FIRST_USER_STACK_TOP - stack_top;
    if offset % crate::user::ring3::USER_STACK_SLOT_SIZE != 0 {
        return None;
    }
    let index = (offset / crate::user::ring3::USER_STACK_SLOT_SIZE) as usize;
    (index < PRIVATE_STACK_SLOT_COUNT).then_some(index)
}

pub fn active_p4_frame() -> u64 {
    let (frame, _) = Cr3::read();
    frame.start_address().as_u64()
}

pub fn kernel_p4_frame() -> Option<u64> {
    *KERNEL_P4_FRAME.lock()
}

pub fn active_user_range_accessible(ptr: u64, len: u64, access: UserPageAccess) -> bool {
    if len == 0 {
        return false;
    }
    let Some(end) = ptr.checked_add(len) else {
        return false;
    };
    if end <= ptr {
        return false;
    }

    let Some(address_space) = crate::user::process::current_user_address_space() else {
        return false;
    };
    if let Some(expected_p4) = address_space.p4_frame {
        if active_p4_frame() != expected_p4 {
            return false;
        }
    }

    let Ok(physical_memory_offset) = physical_memory_offset() else {
        return false;
    };
    let p4_frame = PhysFrame::containing_address(PhysAddr::new(active_p4_frame()));
    let mut page_address = ptr & !(Size4KiB::SIZE - 1);
    let last_page = (end - 1) & !(Size4KiB::SIZE - 1);

    loop {
        // SAFETY: the physical-memory offset comes from bootloader metadata,
        // p4_frame is the active CR3, and page_address is canonical user space.
        let accessible = unsafe {
            active_user_page_accessible(
                physical_memory_offset,
                p4_frame,
                VirtAddr::new(page_address),
                access,
            )
        };
        if !accessible {
            return false;
        }
        if page_address == last_page {
            return true;
        }
        let Some(next) = page_address.checked_add(Size4KiB::SIZE) else {
            return false;
        };
        page_address = next;
    }
}

unsafe fn active_user_page_accessible(
    physical_memory_offset: VirtAddr,
    p4_frame: PhysFrame<Size4KiB>,
    virtual_address: VirtAddr,
    access: UserPageAccess,
) -> bool {
    let p4 = unsafe { page_table_mut(physical_memory_offset, p4_frame) };
    let Ok(p3_frame) = accessible_child_frame(p4, virtual_address.p4_index().into(), access) else {
        return false;
    };
    let p3 = unsafe { page_table_mut(physical_memory_offset, p3_frame) };
    let Ok(p2_frame) = accessible_child_frame(p3, virtual_address.p3_index().into(), access) else {
        return false;
    };
    let p2 = unsafe { page_table_mut(physical_memory_offset, p2_frame) };
    let Ok(p1_frame) = accessible_child_frame(p2, virtual_address.p2_index().into(), access) else {
        return false;
    };
    let p1 = unsafe { page_table_mut(physical_memory_offset, p1_frame) };
    flags_allow_user_access(p1[usize::from(virtual_address.p1_index())].flags(), access)
}

fn accessible_child_frame(
    table: &PageTable,
    index: usize,
    access: UserPageAccess,
) -> Result<PhysFrame<Size4KiB>, AddressSpaceError> {
    if !flags_allow_user_access(table[index].flags(), access) {
        return Err(AddressSpaceError::UserMappingNotAccessible);
    }
    child_table_frame(table, index)
}

fn flags_allow_user_access(flags: PageTableFlags, access: UserPageAccess) -> bool {
    if !flags.contains(PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE) {
        return false;
    }
    !matches!(access, UserPageAccess::Write) || flags.contains(PageTableFlags::WRITABLE)
}

fn physical_memory_offset() -> Result<VirtAddr, AddressSpaceError> {
    let cached = PHYSICAL_MEMORY_OFFSET_VALUE.load(Ordering::Relaxed);
    if cached != 0 {
        return Ok(VirtAddr::new(cached));
    }

    let offset =
        { *PHYSICAL_MEMORY_OFFSET.lock() }.ok_or(AddressSpaceError::MissingPhysicalMemoryOffset)?;
    PHYSICAL_MEMORY_OFFSET_VALUE.store(offset, Ordering::Relaxed);
    Ok(VirtAddr::new(offset))
}

pub fn record_kernel_p4_frame() -> u64 {
    let frame = active_p4_frame();
    *KERNEL_P4_FRAME.lock() = Some(frame);
    frame
}

pub unsafe fn smoke_switch_to_prepared_p4() -> Result<u64, AddressSpaceError> {
    let prepared_frame = PRIVATE_P4_POOL
        .lock()
        .peek_next()
        .ok_or(AddressSpaceError::NoPreparedP4)?;

    unsafe { smoke_switch_to_p4_frame(prepared_frame) }
}

pub unsafe fn smoke_switch_to_p4_frame(prepared_frame: u64) -> Result<u64, AddressSpaceError> {
    let (original_frame, original_flags) = Cr3::read();
    let prepared = PhysFrame::containing_address(PhysAddr::new(prepared_frame));

    unsafe {
        Cr3::write(prepared, original_flags);
    }

    let switched_frame = active_p4_frame();
    unsafe {
        Cr3::write(original_frame, original_flags);
    }

    if switched_frame != prepared_frame {
        return Err(AddressSpaceError::SwitchVerificationFailed);
    }

    Ok(prepared_frame)
}

pub unsafe fn switch_to_p4_frame(frame: u64) -> Result<u64, AddressSpaceError> {
    let (original_frame, flags) = Cr3::read();
    let target = PhysFrame::containing_address(PhysAddr::new(frame));

    unsafe {
        Cr3::write(target, flags);
    }

    let switched_frame = active_p4_frame();
    if switched_frame != frame {
        unsafe {
            Cr3::write(original_frame, flags);
        }
        return Err(AddressSpaceError::SwitchVerificationFailed);
    }

    Ok(switched_frame)
}

pub unsafe fn restore_kernel_p4() -> Result<u64, AddressSpaceError> {
    let frame = kernel_p4_frame().ok_or(AddressSpaceError::NoKernelP4)?;
    unsafe { switch_to_p4_frame(frame) }
}

pub unsafe fn ensure_active_user_program_mapping_writable(
    size: u64,
) -> Result<(), AddressSpaceError> {
    if size == 0 {
        return Ok(());
    }

    let physical_memory_offset = physical_memory_offset()?;
    let active_frame = PhysFrame::containing_address(PhysAddr::new(active_p4_frame()));
    let start = VirtAddr::new(crate::user::ring3::FIRST_USER_ENTRY);
    let size = size.min(crate::user::ring3::USER_PROGRAM_AREA_SIZE);
    let end = VirtAddr::new(crate::user::ring3::FIRST_USER_ENTRY + size - 1);
    let start_page = Page::<Size4KiB>::containing_address(start);
    let end_page = Page::<Size4KiB>::containing_address(end);

    for page in Page::range_inclusive(start_page, end_page) {
        unsafe {
            ensure_user_page_writable(physical_memory_offset, active_frame, page.start_address())?;
        }
    }

    Ok(())
}

pub unsafe fn ensure_active_user_stack_mapping_writable(
    stack_start: u64,
    stack_top: u64,
) -> Result<(), AddressSpaceError> {
    if stack_top <= stack_start {
        return Ok(());
    }

    let physical_memory_offset = physical_memory_offset()?;
    let active_frame = PhysFrame::containing_address(PhysAddr::new(active_p4_frame()));
    let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(stack_start));
    let end_page = Page::<Size4KiB>::containing_address(VirtAddr::new(stack_top - 1));

    for page in Page::range_inclusive(start_page, end_page) {
        unsafe {
            ensure_user_page_writable(physical_memory_offset, active_frame, page.start_address())?;
        }
    }

    Ok(())
}

pub unsafe fn prepare_process_private_memory(
    p4_frame: u64,
    layout: crate::user::ring3::UserMemoryLayout,
    image: &[u8],
    program_path: &'static str,
    arg: &[u8],
) -> Result<(), AddressSpaceError> {
    crate::serial_println!("[USER] entering private memory prepare p4={:#x}", p4_frame);
    let physical_memory_offset = physical_memory_offset()?;
    crate::serial_println!(
        "[USER] private memory lookup p4={:#x} image_bytes={}",
        p4_frame,
        image.len()
    );
    let pool = PRIVATE_P4_POOL.lock();
    crate::serial_println!(
        "[USER] private memory pool locked p4={:#x} slots={}",
        p4_frame,
        pool.count
    );
    let slot_index = pool
        .find_index(p4_frame)
        .ok_or(AddressSpaceError::PrivateMappingNotFound)?;
    let slot = &pool.slots[slot_index];
    let program_page_count =
        pages_for_len(layout.program_end.saturating_sub(layout.program_start) as usize);

    if program_page_count > slot.program_frame_count {
        return Err(AddressSpaceError::PrivateMappingTooSmall);
    }

    let stack_page_count =
        pages_for_len(layout.stack_top.saturating_sub(layout.stack_start) as usize);
    let stack_slot =
        stack_slot_index(layout.stack_top).ok_or(AddressSpaceError::PrivateMappingNotFound)?;
    if stack_page_count > PRIVATE_STACK_PAGE_COUNT {
        return Err(AddressSpaceError::PrivateMappingTooSmall);
    }

    crate::serial_println!(
        "[USER] private memory prep p4={:#x} program_pages={} stack_pages={} mem={:#x}-{:#x} stack={:#x}-{:#x}",
        p4_frame,
        program_page_count,
        stack_page_count,
        layout.program_start,
        layout.program_end,
        layout.stack_start,
        layout.stack_top
    );

    for index in 0..stack_page_count {
        unsafe {
            remap_existing_user_page_to_private_frame(
                physical_memory_offset,
                PhysFrame::containing_address(PhysAddr::new(p4_frame)),
                VirtAddr::new(layout.stack_start + (index as u64 * Size4KiB::SIZE)),
                PhysFrame::containing_address(PhysAddr::new(slot.stack_frames[stack_slot][index])),
            )?;
        }
    }

    let is_elf_image = crate::user::elf::parse_elf64(image).is_ok();
    for index in 0..program_page_count {
        let destination = (physical_memory_offset + slot.program_frames[index]).as_mut_ptr::<u8>();
        unsafe {
            if is_elf_image {
                core::ptr::write_bytes(destination, 0, Size4KiB::SIZE as usize);
            } else {
                let source = (crate::user::ring3::FIRST_USER_ENTRY
                    + (index as u64 * Size4KiB::SIZE)) as *const u8;
                core::ptr::copy_nonoverlapping(source, destination, Size4KiB::SIZE as usize);
            }
        }
    }

    for index in 0..stack_page_count {
        let stack_destination =
            (physical_memory_offset + slot.stack_frames[stack_slot][index]).as_mut_ptr::<u8>();
        unsafe {
            core::ptr::write_bytes(stack_destination, 0, Size4KiB::SIZE as usize);
        }
    }

    unsafe {
        overlay_elf_segments_to_private_frames(physical_memory_offset, slot, image)?;
        seed_private_user_arg(physical_memory_offset, slot, arg)?;
        seed_private_initial_stack(physical_memory_offset, slot, layout, program_path, arg)?;
        protect_private_user_memory(
            physical_memory_offset,
            PhysFrame::containing_address(PhysAddr::new(p4_frame)),
            layout,
            image,
        )?;
    }

    drop(pool);
    let mut pool = PRIVATE_P4_POOL.lock();
    let count = pool.count;
    if let Some(slot) = pool.slots[..count]
        .iter_mut()
        .find(|slot| slot.p4_frame == p4_frame)
    {
        slot.stack_in_use[stack_slot] = true;
    }

    Ok(())
}

unsafe fn protect_private_user_memory(
    physical_memory_offset: VirtAddr,
    p4_frame: PhysFrame<Size4KiB>,
    layout: crate::user::ring3::UserMemoryLayout,
    image: &[u8],
) -> Result<(), AddressSpaceError> {
    let mut segments = [crate::user::elf::LoadSegment::EMPTY; 8];
    let segment_count = crate::user::elf::load_segments(image, &mut segments).ok();

    let program_start = Page::<Size4KiB>::containing_address(VirtAddr::new(layout.program_start));
    let program_end =
        Page::<Size4KiB>::containing_address(VirtAddr::new(layout.program_end.saturating_sub(1)));
    for page in Page::range_inclusive(program_start, program_end) {
        let flags = match segment_count {
            Some(count) => elf_page_flags(page.start_address().as_u64(), &segments[..count])?,
            None => crate::user::ring3::user_page_flags(),
        };
        unsafe {
            set_user_page_flags(
                physical_memory_offset,
                p4_frame,
                page.start_address(),
                flags,
            )?;
        }
    }

    let stack_start = Page::<Size4KiB>::containing_address(VirtAddr::new(layout.stack_start));
    let stack_end =
        Page::<Size4KiB>::containing_address(VirtAddr::new(layout.stack_top.saturating_sub(1)));
    for page in Page::range_inclusive(stack_start, stack_end) {
        unsafe {
            set_user_page_flags(
                physical_memory_offset,
                p4_frame,
                page.start_address(),
                crate::user::ring3::user_data_page_flags(),
            )?;
        }
    }

    Ok(())
}

fn elf_page_flags(
    page_start: u64,
    segments: &[crate::user::elf::LoadSegment],
) -> Result<PageTableFlags, AddressSpaceError> {
    let page_end = page_start.saturating_add(Size4KiB::SIZE);
    let mut matched = false;
    let mut writable = false;
    let mut executable = false;

    for segment in segments {
        let segment_end = segment.virtual_address.saturating_add(segment.memory_size);
        if segment.virtual_address < page_end && segment_end > page_start {
            matched = true;
            writable |= segment.flags & crate::user::elf::PF_W != 0;
            executable |= segment.flags & crate::user::elf::PF_X != 0;
        }
    }

    if writable && executable {
        return Err(AddressSpaceError::UserMappingNotAccessible);
    }
    if !matched {
        return Ok(crate::user::ring3::user_data_page_flags());
    }

    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
    if !executable {
        flags |= PageTableFlags::NO_EXECUTE;
    }
    Ok(flags)
}

unsafe fn set_user_page_flags(
    physical_memory_offset: VirtAddr,
    p4_frame: PhysFrame<Size4KiB>,
    virtual_address: VirtAddr,
    flags: PageTableFlags,
) -> Result<(), AddressSpaceError> {
    let p4 = unsafe { page_table_mut(physical_memory_offset, p4_frame) };
    let p3_frame = child_table_frame(p4, virtual_address.p4_index().into())?;
    let p3 = unsafe { page_table_mut(physical_memory_offset, p3_frame) };
    let p2_frame = child_table_frame(p3, virtual_address.p3_index().into())?;
    let p2 = unsafe { page_table_mut(physical_memory_offset, p2_frame) };
    let p1_frame = child_table_frame(p2, virtual_address.p2_index().into())?;
    let p1 = unsafe { page_table_mut(physical_memory_offset, p1_frame) };
    p1[usize::from(virtual_address.p1_index())].set_flags(flags);
    Ok(())
}

unsafe fn seed_private_user_arg(
    physical_memory_offset: VirtAddr,
    slot: &PreparedP4Slot,
    arg: &[u8],
) -> Result<(), AddressSpaceError> {
    let arg_len = (arg.len() as u64).min(crate::user::ring3::FIRST_USER_ARG_PATH_LEN);
    let arg = &arg[..arg_len as usize];

    unsafe {
        zero_private_user_bytes(
            physical_memory_offset,
            slot,
            crate::user::ring3::FIRST_USER_ARG_PATH_ADDR,
            crate::user::ring3::FIRST_USER_ARG_PATH_LEN,
        )?;
        write_private_user_bytes(
            physical_memory_offset,
            slot,
            crate::user::ring3::FIRST_USER_ARG_PATH_ADDR,
            arg,
        )?;
        write_private_user_bytes(
            physical_memory_offset,
            slot,
            crate::user::ring3::FIRST_USER_ARG_LEN_ADDR,
            &arg_len.to_le_bytes(),
        )?;
    }

    Ok(())
}

unsafe fn seed_private_initial_stack(
    physical_memory_offset: VirtAddr,
    slot: &PreparedP4Slot,
    layout: crate::user::ring3::UserMemoryLayout,
    program_path: &str,
    arg: &[u8],
) -> Result<(), AddressSpaceError> {
    let stack = crate::user::ring3::build_user_initial_stack(layout, program_path, arg)
        .map_err(|_| AddressSpaceError::PrivateMappingTooSmall)?;

    unsafe {
        write_private_user_stack_bytes(
            physical_memory_offset,
            slot,
            layout,
            stack.stack_pointer,
            stack.as_bytes(),
        )?;
    }

    Ok(())
}

unsafe fn overlay_elf_segments_to_private_frames(
    physical_memory_offset: VirtAddr,
    slot: &PreparedP4Slot,
    image: &[u8],
) -> Result<(), AddressSpaceError> {
    let mut segments = [crate::user::elf::LoadSegment::EMPTY; 8];
    let Ok(count) = crate::user::elf::load_segments(image, &mut segments) else {
        return Ok(());
    };

    for segment in &segments[..count] {
        unsafe {
            write_private_user_bytes(
                physical_memory_offset,
                slot,
                segment.virtual_address,
                &image[segment.file_offset as usize
                    ..segment.file_offset.saturating_add(segment.file_size) as usize],
            )?;
        }

        if segment.memory_size > segment.file_size {
            unsafe {
                zero_private_user_bytes(
                    physical_memory_offset,
                    slot,
                    segment.virtual_address + segment.file_size,
                    segment.memory_size - segment.file_size,
                )?;
            }
        }
    }

    Ok(())
}

unsafe fn write_private_user_bytes(
    physical_memory_offset: VirtAddr,
    slot: &PreparedP4Slot,
    mut virtual_address: u64,
    mut bytes: &[u8],
) -> Result<(), AddressSpaceError> {
    while !bytes.is_empty() {
        let page_index = private_program_page_index(virtual_address, slot)?;
        let page_offset = (virtual_address - crate::user::ring3::FIRST_USER_ENTRY) % Size4KiB::SIZE;
        let count = bytes
            .len()
            .min(Size4KiB::SIZE as usize - page_offset as usize);
        let destination = (physical_memory_offset + slot.program_frames[page_index] + page_offset)
            .as_mut_ptr::<u8>();
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, count);
        }
        virtual_address += count as u64;
        bytes = &bytes[count..];
    }

    Ok(())
}

unsafe fn zero_private_user_bytes(
    physical_memory_offset: VirtAddr,
    slot: &PreparedP4Slot,
    mut virtual_address: u64,
    mut len: u64,
) -> Result<(), AddressSpaceError> {
    while len > 0 {
        let page_index = private_program_page_index(virtual_address, slot)?;
        let page_offset = (virtual_address - crate::user::ring3::FIRST_USER_ENTRY) % Size4KiB::SIZE;
        let count = len.min(Size4KiB::SIZE - page_offset);
        let destination = (physical_memory_offset + slot.program_frames[page_index] + page_offset)
            .as_mut_ptr::<u8>();
        unsafe {
            core::ptr::write_bytes(destination, 0, count as usize);
        }
        virtual_address += count;
        len -= count;
    }

    Ok(())
}

unsafe fn write_private_user_stack_bytes(
    physical_memory_offset: VirtAddr,
    slot: &PreparedP4Slot,
    layout: crate::user::ring3::UserMemoryLayout,
    mut virtual_address: u64,
    mut bytes: &[u8],
) -> Result<(), AddressSpaceError> {
    let stack_slot =
        stack_slot_index(layout.stack_top).ok_or(AddressSpaceError::PrivateMappingNotFound)?;
    while !bytes.is_empty() {
        let page_index = private_stack_page_index(virtual_address, layout)?;
        let page_offset = (virtual_address - layout.stack_start) % Size4KiB::SIZE;
        let count = bytes
            .len()
            .min(Size4KiB::SIZE as usize - page_offset as usize);
        let destination =
            (physical_memory_offset + slot.stack_frames[stack_slot][page_index] + page_offset)
                .as_mut_ptr::<u8>();
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, count);
        }
        virtual_address += count as u64;
        bytes = &bytes[count..];
    }

    Ok(())
}

fn private_program_page_index(
    virtual_address: u64,
    slot: &PreparedP4Slot,
) -> Result<usize, AddressSpaceError> {
    if virtual_address < crate::user::ring3::FIRST_USER_ENTRY {
        return Err(AddressSpaceError::PrivateMappingNotFound);
    }
    let index =
        ((virtual_address - crate::user::ring3::FIRST_USER_ENTRY) / Size4KiB::SIZE) as usize;
    if index >= slot.program_frame_count {
        return Err(AddressSpaceError::PrivateMappingTooSmall);
    }
    Ok(index)
}

fn private_stack_page_index(
    virtual_address: u64,
    layout: crate::user::ring3::UserMemoryLayout,
) -> Result<usize, AddressSpaceError> {
    if virtual_address < layout.stack_start || virtual_address >= layout.stack_top {
        return Err(AddressSpaceError::PrivateMappingNotFound);
    }
    let index = ((virtual_address - layout.stack_start) / Size4KiB::SIZE) as usize;
    if index >= PRIVATE_STACK_PAGE_COUNT {
        return Err(AddressSpaceError::PrivateMappingTooSmall);
    }
    Ok(index)
}

unsafe fn ensure_user_page_writable(
    physical_memory_offset: VirtAddr,
    p4_frame: PhysFrame<Size4KiB>,
    virtual_address: VirtAddr,
) -> Result<(), AddressSpaceError> {
    let p4 = unsafe { page_table_mut(physical_memory_offset, p4_frame) };
    let p3_frame = child_table_frame(p4, virtual_address.p4_index().into())?;
    let p3 = unsafe { page_table_mut(physical_memory_offset, p3_frame) };
    let p2_frame = child_table_frame(p3, virtual_address.p3_index().into())?;
    let p2 = unsafe { page_table_mut(physical_memory_offset, p2_frame) };
    let p1_frame = child_table_frame(p2, virtual_address.p2_index().into())?;
    let p1 = unsafe { page_table_mut(physical_memory_offset, p1_frame) };
    let entry = &mut p1[usize::from(virtual_address.p1_index())];
    let flags = entry.flags() | crate::user::ring3::user_data_page_flags();
    entry.set_flags(flags);
    tlb::flush(virtual_address);

    Ok(())
}

unsafe fn clone_active_p4_into_frame(physical_memory_offset: VirtAddr, frame: PhysFrame<Size4KiB>) {
    let (active_frame, _) = Cr3::read();
    let active_virt = physical_memory_offset + active_frame.start_address().as_u64();
    let destination_virt = physical_memory_offset + frame.start_address().as_u64();
    let active: *const PageTable = active_virt.as_ptr();
    let destination: *mut PageTable = destination_virt.as_mut_ptr();

    unsafe {
        core::ptr::copy_nonoverlapping(active, destination, 1);
    }
}

unsafe fn prepare_private_p4_slot(
    physical_memory_offset: VirtAddr,
    p4_frame: PhysFrame<Size4KiB>,
    program_page_count: usize,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<PreparedP4Slot, AddressSpaceError> {
    unsafe {
        clone_active_p4_into_frame(physical_memory_offset, p4_frame);
    }

    let mut program_frames = [0u64; MAX_PRIVATE_PROGRAM_PAGE_COUNT];
    for (index, frame_addr) in program_frames
        .iter_mut()
        .enumerate()
        .take(program_page_count)
    {
        let private_frame = frame_allocator
            .allocate_frame()
            .ok_or(AddressSpaceError::FrameAllocationFailed)?;
        unsafe {
            map_private_user_page(
                physical_memory_offset,
                p4_frame,
                VirtAddr::new(
                    crate::user::ring3::FIRST_USER_ENTRY + (index as u64 * Size4KiB::SIZE),
                ),
                private_frame,
                frame_allocator,
            )?;
        }
        *frame_addr = private_frame.start_address().as_u64();
    }

    let mut stack_frames = [[0u64; PRIVATE_STACK_PAGE_COUNT]; PRIVATE_STACK_SLOT_COUNT];
    for (slot_index, slot_frames) in stack_frames.iter_mut().enumerate() {
        let (stack_start, _) = stack_range_for_slot(slot_index);
        for (page_index, frame_addr) in slot_frames.iter_mut().enumerate() {
            let stack_frame = frame_allocator
                .allocate_frame()
                .ok_or(AddressSpaceError::FrameAllocationFailed)?;
            unsafe {
                map_private_user_page(
                    physical_memory_offset,
                    p4_frame,
                    VirtAddr::new(stack_start + page_index as u64 * Size4KiB::SIZE),
                    stack_frame,
                    frame_allocator,
                )?;
            }
            *frame_addr = stack_frame.start_address().as_u64();
        }
    }

    Ok(PreparedP4Slot {
        p4_frame: p4_frame.start_address().as_u64(),
        program_frames,
        program_frame_count: program_page_count,
        stack_frames,
        stack_in_use: [false; PRIVATE_STACK_SLOT_COUNT],
        in_use: false,
    })
}

unsafe fn map_private_user_page(
    physical_memory_offset: VirtAddr,
    p4_frame: PhysFrame<Size4KiB>,
    virtual_address: VirtAddr,
    private_frame: PhysFrame<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), AddressSpaceError> {
    let p4 = unsafe { page_table_mut(physical_memory_offset, p4_frame) };
    let p3_frame = clone_child_table(
        physical_memory_offset,
        p4,
        virtual_address.p4_index().into(),
        frame_allocator,
    )?;
    let p3 = unsafe { page_table_mut(physical_memory_offset, p3_frame) };
    let p2_frame = clone_child_table(
        physical_memory_offset,
        p3,
        virtual_address.p3_index().into(),
        frame_allocator,
    )?;
    let p2 = unsafe { page_table_mut(physical_memory_offset, p2_frame) };
    let p1_frame = clone_child_table(
        physical_memory_offset,
        p2,
        virtual_address.p2_index().into(),
        frame_allocator,
    )?;
    let p1 = unsafe { page_table_mut(physical_memory_offset, p1_frame) };

    p1[usize::from(virtual_address.p1_index())]
        .set_frame(private_frame, crate::user::ring3::user_data_page_flags());

    Ok(())
}

unsafe fn remap_existing_user_page_to_private_frame(
    physical_memory_offset: VirtAddr,
    p4_frame: PhysFrame<Size4KiB>,
    virtual_address: VirtAddr,
    private_frame: PhysFrame<Size4KiB>,
) -> Result<(), AddressSpaceError> {
    let p4 = unsafe { page_table_mut(physical_memory_offset, p4_frame) };
    let p3_frame = child_table_frame(p4, virtual_address.p4_index().into())?;
    let p3 = unsafe { page_table_mut(physical_memory_offset, p3_frame) };
    let p2_frame = child_table_frame(p3, virtual_address.p3_index().into())?;
    let p2 = unsafe { page_table_mut(physical_memory_offset, p2_frame) };
    let p1_frame = child_table_frame(p2, virtual_address.p2_index().into())?;
    let p1 = unsafe { page_table_mut(physical_memory_offset, p1_frame) };

    p1[usize::from(virtual_address.p1_index())]
        .set_frame(private_frame, crate::user::ring3::user_data_page_flags());

    Ok(())
}

fn child_table_frame(
    table: &PageTable,
    index: usize,
) -> Result<PhysFrame<Size4KiB>, AddressSpaceError> {
    table[index].frame().map_err(|err| match err {
        FrameError::FrameNotPresent => AddressSpaceError::PrivateMappingNotFound,
        FrameError::HugeFrame => AddressSpaceError::PrivateMappingNotFound,
    })
}

fn clone_child_table(
    physical_memory_offset: VirtAddr,
    parent: &mut PageTable,
    index: usize,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<PhysFrame<Size4KiB>, AddressSpaceError> {
    let current_frame = child_table_frame(parent, index)?;
    let new_frame = frame_allocator
        .allocate_frame()
        .ok_or(AddressSpaceError::FrameAllocationFailed)?;

    let source = physical_memory_offset + current_frame.start_address().as_u64();
    let destination = physical_memory_offset + new_frame.start_address().as_u64();
    unsafe {
        core::ptr::copy_nonoverlapping(
            source.as_ptr::<PageTable>(),
            destination.as_mut_ptr::<PageTable>(),
            1,
        );
    }

    let flags = parent[index].flags() | PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
    parent[index].set_frame(new_frame, flags);

    Ok(new_frame)
}

unsafe fn page_table_mut(
    physical_memory_offset: VirtAddr,
    frame: PhysFrame<Size4KiB>,
) -> &'static mut PageTable {
    let virt = physical_memory_offset + frame.start_address().as_u64();
    // SAFETY: callers guarantee that `frame` names a live page-table frame and
    // that the bootloader physical-memory mapping covers its full 4 KiB page.
    unsafe { &mut *virt.as_mut_ptr::<PageTable>() }
}

fn pages_for_len(len: usize) -> usize {
    len.div_ceil(Size4KiB::SIZE as usize)
}

fn required_private_program_pages() -> usize {
    let mut pages = 1usize;

    for image in crate::user::images::USERLAND_IMAGES {
        pages = pages.max(image_private_page_count(image.data));
    }

    pages.clamp(1, MAX_PRIVATE_PROGRAM_PAGE_COUNT)
}

fn image_private_page_count(image: &[u8]) -> usize {
    if crate::user::elf::parse_elf64(image).is_ok() {
        let mut segments = [crate::user::elf::LoadSegment::EMPTY; 8];
        if let Ok(count) = crate::user::elf::load_segments(image, &mut segments) {
            let footprint = crate::user::elf::image_footprint_size(&segments[..count]);
            return pages_for_len(footprint as usize);
        }
    }

    pages_for_len(image.len())
}

#[cfg(test)]
mod tests {
    use super::{
        AddressSpace, AddressSpaceKind, PreparedP4Slot, PrivateP4Pool, UserPageAccess,
        flags_allow_user_access, image_private_page_count,
    };
    use x86_64::structures::paging::PageTableFlags;

    #[test_case]
    fn user_page_access_requires_present_and_user_flags() {
        let valid = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;

        assert!(flags_allow_user_access(valid, UserPageAccess::Read));
        assert!(!flags_allow_user_access(
            PageTableFlags::PRESENT,
            UserPageAccess::Read
        ));
        assert!(!flags_allow_user_access(
            PageTableFlags::USER_ACCESSIBLE,
            UserPageAccess::Read
        ));
    }

    #[test_case]
    fn user_page_write_requires_writable_at_every_level() {
        let read_only = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        let writable = read_only | PageTableFlags::WRITABLE;

        assert!(!flags_allow_user_access(read_only, UserPageAccess::Write));
        assert!(flags_allow_user_access(writable, UserPageAccess::Write));
    }

    #[test_case]
    fn kernel_shared_user_has_no_private_page_table_yet() {
        let layout = crate::user::ring3::memory_layout_for_pid(2);
        let space = AddressSpace::kernel_shared_user(crate::user::ring3::FIRST_USER_ENTRY, layout);

        assert_eq!(space.kind, AddressSpaceKind::KernelSharedUser);
        assert_eq!(space.p4_frame, None);
        assert!(!space.has_verified_p4());
        assert!(!space.is_isolated());
        assert_eq!(
            space.user_stack_top(),
            crate::user::ring3::FIRST_USER_STACK_TOP
        );
    }

    #[test_case]
    fn isolated_user_records_p4_frame() {
        let layout = crate::user::ring3::memory_layout_for_pid(3);
        let space = AddressSpace::isolated_user(0x401000, layout, 0x12345000);

        assert_eq!(space.kind, AddressSpaceKind::IsolatedUser);
        assert_eq!(space.p4_frame, Some(0x12345000));
        assert!(space.has_verified_p4());
        assert!(space.is_isolated());
        assert_eq!(space.user_stack_top(), 0x7ff000);
    }

    #[test_case]
    fn prepared_user_records_p4_without_claiming_isolation() {
        let layout = crate::user::ring3::memory_layout_for_pid(4);
        let space = AddressSpace::prepared_user(0x400000, layout, 0x200000);

        assert_eq!(space.kind, AddressSpaceKind::PreparedUser);
        assert_eq!(space.p4_frame, Some(0x200000));
        assert!(!space.has_verified_p4());
        assert!(!space.is_isolated());
    }

    #[test_case]
    fn verified_prepared_user_records_checked_p4_without_claiming_isolation() {
        let layout = crate::user::ring3::memory_layout_for_pid(4);
        let space = AddressSpace::verified_prepared_user(0x400000, layout, 0x200000);

        assert_eq!(space.kind, AddressSpaceKind::PreparedUser);
        assert_eq!(space.p4_frame, Some(0x200000));
        assert!(space.has_verified_p4());
        assert!(!space.is_isolated());
    }

    #[test_case]
    fn verified_prepared_user_can_be_promoted_to_isolated() {
        let layout = crate::user::ring3::memory_layout_for_pid(4);
        let space = AddressSpace::verified_prepared_user(0x400000, layout, 0x200000)
            .promoted_isolated_user();

        assert_eq!(space.kind, AddressSpaceKind::IsolatedUser);
        assert_eq!(space.p4_frame, Some(0x200000));
        assert!(space.has_verified_p4());
        assert!(space.is_isolated());
    }

    #[test_case]
    fn flat_user_image_uses_at_least_one_private_page() {
        assert_eq!(image_private_page_count(&[0xcc; 145]), 1);
    }

    #[test_case]
    fn private_p4_pool_reuses_released_slots() {
        let mut pool = PrivateP4Pool::new();
        let mut slot = PreparedP4Slot::EMPTY;
        slot.p4_frame = 0x2000;
        assert!(pool.push(slot));

        assert_eq!(pool.take(), Some(0x2000));
        assert_eq!(pool.take(), None);
        assert!(pool.release(0x2000));
        assert_eq!(pool.take(), Some(0x2000));
    }

    #[test_case]
    fn private_p4_pool_reserves_and_reuses_non_main_stack_slots() {
        let mut pool = PrivateP4Pool::new();
        let mut slot = PreparedP4Slot::EMPTY;
        slot.p4_frame = 0x2000;
        slot.in_use = true;
        assert!(pool.push(slot));

        let main_top = crate::user::ring3::stack_top_for_pid(5);
        let first = pool.reserve_stack_slot(0x2000, main_top).unwrap();
        assert_ne!(first.1, main_top);
        assert!(pool.release_stack_slot(0x2000, first.1));
        assert_eq!(
            pool.reserve_stack_slot(0x2000, main_top),
            Some(first),
            "released stack slot should be reused"
        );
    }
}
