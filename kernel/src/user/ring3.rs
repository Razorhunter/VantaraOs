use core::arch::asm;

use x86_64::{
    VirtAddr,
    structures::paging::{
        FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB, mapper::MapToError,
    },
};

use crate::gdt;

pub const USER_BASE: u64 = 0x0000_0000_0100_0000;
pub const FIRST_USER_ENTRY: u64 = USER_BASE;
pub const FIRST_USER_STACK_TOP: u64 = USER_BASE + 0x0040_0000;
pub const USER_STACK_PAGE_COUNT: u64 = 4;
pub const USER_STACK_GUARD_SIZE: u64 = 4096;
pub const USER_STACK_SIZE: u64 = 4096 * USER_STACK_PAGE_COUNT;
pub const USER_STACK_SLOT_SIZE: u64 = USER_STACK_SIZE + USER_STACK_GUARD_SIZE;
pub const USER_STACK_SLOT_COUNT: u64 = 8;
pub const USER_STACK_REGION_SIZE: u64 = USER_STACK_SLOT_SIZE * USER_STACK_SLOT_COUNT;
pub const USER_STACK_REGION_START: u64 = FIRST_USER_STACK_TOP - USER_STACK_REGION_SIZE;
pub const USER_PROGRAM_AREA_SIZE: u64 = USER_STACK_REGION_START - FIRST_USER_ENTRY;
pub const USER_INITIAL_STACK_MAX_SIZE: usize = 512;
pub const USER_INITIAL_ARGV_MAX: usize = 8;
pub const FIRST_USER_PAGE_SIZE: usize = 4096;
pub const FIRST_USER_DATA_OFFSET: usize = 0x300;
pub const FIRST_USER_MESSAGE_ADDR: u64 = FIRST_USER_ENTRY + 0x300;
pub const FIRST_USER_MESSAGE: &[u8] = b"uptime ms: ";
pub const FIRST_USER_NEWLINE_ADDR: u64 = FIRST_USER_ENTRY + 0x30b;
pub const FIRST_USER_UPTIME_BUFFER_ADDR: u64 = FIRST_USER_ENTRY + 0x320;
pub const FIRST_USER_UPTIME_BUFFER_LEN: u64 = 32;
pub const FIRST_USER_ARG_PATH_ADDR: u64 = FIRST_USER_ENTRY + 0x340;
pub const FIRST_USER_ARG_PATH_LEN: u64 = 32;
pub const FIRST_USER_LS_HEADER_ADDR: u64 = FIRST_USER_ENTRY + 0x360;
pub const FIRST_USER_LS_HEADER: &[u8] = b"ls /:\n";
pub const FIRST_USER_ROOT_PATH_ADDR: u64 = FIRST_USER_ENTRY + 0x366;
pub const FIRST_USER_CAT_PATH: &[u8] = b"/README";
pub const FIRST_USER_ARG_LEN_ADDR: u64 = FIRST_USER_ENTRY + 0x378;
pub const FIRST_USER_LIST_BUFFER_ADDR: u64 = FIRST_USER_ENTRY + 0x380;
pub const FIRST_USER_LIST_BUFFER_LEN: u64 = 128;
pub const FIRST_USER_ERROR_MSG_ADDR: u64 = FIRST_USER_ENTRY + 0x3f0;
pub const FIRST_USER_CAT_ERROR: &[u8] = b"cat: not found\n";
pub const FIRST_USER_LS_ERROR: &[u8] = b"ls: not found\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserMemoryLayout {
    pub program_start: u64,
    pub program_end: u64,
    pub stack_start: u64,
    pub stack_top: u64,
}

impl UserMemoryLayout {
    pub const fn kernel_placeholder() -> Self {
        Self {
            program_start: 0,
            program_end: 0,
            stack_start: 0,
            stack_top: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserImageError {
    EmptyImage,
    ImageTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstUserTask {
    pub entry: u64,
    pub stack_top: u64,
    pub image_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserInitialStack {
    pub stack_pointer: u64,
    pub bytes: [u8; USER_INITIAL_STACK_MAX_SIZE],
    pub len: usize,
}

impl UserInitialStack {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

pub fn first_user_task() -> FirstUserTask {
    FirstUserTask {
        entry: FIRST_USER_ENTRY,
        stack_top: FIRST_USER_STACK_TOP,
        image_len: crate::user::images::default_image()
            .map(|image| image.data.len())
            .unwrap_or(0),
    }
}

pub fn user_page_flags() -> PageTableFlags {
    PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE
}

pub fn user_data_page_flags() -> PageTableFlags {
    user_page_flags() | PageTableFlags::NO_EXECUTE
}

pub fn user_code_page_flags() -> PageTableFlags {
    PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE
}

pub fn map_user_stack(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), MapToError<Size4KiB>> {
    for slot in 0..USER_STACK_SLOT_COUNT {
        let stack_top = FIRST_USER_STACK_TOP - (slot * USER_STACK_SLOT_SIZE);
        let stack_start = VirtAddr::new(stack_top - USER_STACK_SIZE);
        let stack_end = VirtAddr::new(stack_top - 1);
        let start_page = Page::containing_address(stack_start);
        let end_page = Page::containing_address(stack_end);

        for page in Page::range_inclusive(start_page, end_page) {
            let frame = frame_allocator
                .allocate_frame()
                .ok_or(MapToError::FrameAllocationFailed)?;
            unsafe {
                match mapper.map_to(page, frame, user_data_page_flags(), frame_allocator) {
                    Ok(flush) => flush.flush(),
                    Err(MapToError::PageAlreadyMapped(frame)) => {
                        mapper
                            .update_flags(page, user_data_page_flags())
                            .map_err(|_| MapToError::PageAlreadyMapped(frame))?
                            .flush();
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }

    Ok(())
}

pub fn stack_top_for_pid(pid: crate::user::process::Pid) -> u64 {
    let slot = u64::from(pid.saturating_sub(2)) % USER_STACK_SLOT_COUNT;
    FIRST_USER_STACK_TOP - (slot * USER_STACK_SLOT_SIZE)
}

pub fn memory_layout_for_pid(pid: crate::user::process::Pid) -> UserMemoryLayout {
    memory_layout_for_pid_with_program_size(pid, USER_PROGRAM_AREA_SIZE)
}

pub fn memory_layout_for_pid_with_program_size(
    pid: crate::user::process::Pid,
    program_size: u64,
) -> UserMemoryLayout {
    let stack_top = stack_top_for_pid(pid);
    let program_size = align_user_program_size(program_size);
    UserMemoryLayout {
        program_start: FIRST_USER_ENTRY,
        program_end: FIRST_USER_ENTRY + program_size,
        stack_start: stack_top - USER_STACK_SIZE,
        stack_top,
    }
}

fn align_user_program_size(size: u64) -> u64 {
    let size = size.max(FIRST_USER_PAGE_SIZE as u64);
    let aligned = size.div_ceil(FIRST_USER_PAGE_SIZE as u64) * FIRST_USER_PAGE_SIZE as u64;
    aligned.min(USER_PROGRAM_AREA_SIZE)
}

pub fn map_first_user_task(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), MapToError<Size4KiB>> {
    map_initial_user_program_page(mapper, frame_allocator)?;
    map_user_stack(mapper, frame_allocator)?;
    unsafe {
        let image = crate::user::images::default_image()
            .map(|image| image.data)
            .unwrap_or(&[0xeb, 0xfe]);
        load_first_user_task_image(image).map_err(|_| MapToError::FrameAllocationFailed)?;
    }
    Ok(())
}

pub fn map_initial_user_program_page(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), MapToError<Size4KiB>> {
    let page = Page::containing_address(VirtAddr::new(FIRST_USER_ENTRY));
    let frame = frame_allocator
        .allocate_frame()
        .ok_or(MapToError::FrameAllocationFailed)?;

    unsafe {
        match mapper.map_to(page, frame, user_page_flags(), frame_allocator) {
            Ok(flush) => flush.flush(),
            Err(MapToError::PageAlreadyMapped(frame)) => {
                mapper
                    .update_flags(page, user_page_flags())
                    .map_err(|_| MapToError::PageAlreadyMapped(frame))?
                    .flush();
            }
            Err(err) => return Err(err),
        }
    }

    Ok(())
}

pub fn map_user_program_area(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), MapToError<Size4KiB>> {
    crate::memory::map_range(
        VirtAddr::new(FIRST_USER_ENTRY),
        USER_PROGRAM_AREA_SIZE,
        user_page_flags(),
        mapper,
        frame_allocator,
    )
}

pub fn map_user_elf_segments(
    image: &[u8],
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<usize, MapToError<Size4KiB>> {
    let mut segments = [crate::user::elf::LoadSegment::EMPTY; 8];
    let count = crate::user::elf::load_segments(image, &mut segments)
        .map_err(|_| MapToError::FrameAllocationFailed)?;

    for segment in &segments[..count] {
        let flags = crate::user::elf::segment_page_flags(*segment)
            .map_err(|_| MapToError::FrameAllocationFailed)?;
        crate::memory::map_range(
            VirtAddr::new(segment.virtual_address),
            segment.memory_size,
            flags,
            mapper,
            frame_allocator,
        )?;
    }

    Ok(count)
}

pub unsafe fn clear_user_program_area() {
    unsafe {
        core::ptr::write_bytes(FIRST_USER_ENTRY as *mut u8, 0, FIRST_USER_PAGE_SIZE);
    }
}

pub unsafe fn clear_user_stack_slot(stack_top: u64) -> Result<(), UserImageError> {
    if stack_top > FIRST_USER_STACK_TOP
        || stack_top < USER_STACK_REGION_START + USER_STACK_GUARD_SIZE + USER_STACK_SIZE
    {
        return Err(UserImageError::ImageTooLarge);
    }

    unsafe {
        core::ptr::write_bytes(
            (stack_top - USER_STACK_SIZE) as *mut u8,
            0,
            USER_STACK_SIZE as usize,
        );
    }

    Ok(())
}

pub fn build_user_initial_stack(
    layout: UserMemoryLayout,
    program_path: &str,
    arg: &[u8],
) -> Result<UserInitialStack, UserImageError> {
    if layout.stack_top <= layout.stack_start
        || layout.stack_top - layout.stack_start < USER_INITIAL_STACK_MAX_SIZE as u64
    {
        return Err(UserImageError::ImageTooLarge);
    }

    let mut buffer = [0u8; USER_INITIAL_STACK_MAX_SIZE];
    let stack_base = layout.stack_top - USER_INITIAL_STACK_MAX_SIZE as u64;
    let mut low = USER_INITIAL_STACK_MAX_SIZE;
    let mut argv = [b"" as &[u8]; USER_INITIAL_ARGV_MAX];
    let mut argv_ptrs = [0u64; USER_INITIAL_ARGV_MAX];
    let mut argc = 0usize;

    if program_path.is_empty() {
        return Err(UserImageError::EmptyImage);
    }
    argv[argc] = program_path.as_bytes();
    argc += 1;

    let mut index = 0usize;
    while index < arg.len() && argc < USER_INITIAL_ARGV_MAX {
        while index < arg.len() && is_arg_separator(arg[index]) {
            index += 1;
        }
        let start = index;
        while index < arg.len() && !is_arg_separator(arg[index]) {
            index += 1;
        }
        if start < index {
            argv[argc] = &arg[start..index];
            argc += 1;
        }
    }

    for index in (0..argc).rev() {
        let item = argv[index];
        let needed = item
            .len()
            .checked_add(1)
            .ok_or(UserImageError::ImageTooLarge)?;
        if needed > low {
            return Err(UserImageError::ImageTooLarge);
        }
        low -= needed;
        buffer[low..low + item.len()].copy_from_slice(item);
        buffer[low + item.len()] = 0;
        argv_ptrs[index] = stack_base + low as u64;
    }

    low &= !7;

    let word_count = 1 + argc + 1 + 1 + 2;
    if word_count * 8 > low {
        return Err(UserImageError::ImageTooLarge);
    }
    let mut words_start = low - (word_count * 8);
    if (stack_base + words_start as u64) % 16 != 0 {
        if words_start < 8 {
            return Err(UserImageError::ImageTooLarge);
        }
        words_start -= 8;
    }

    low = words_start;
    write_stack_word(&mut buffer, &mut low, argc as u64)?;
    for ptr in &argv_ptrs[..argc] {
        write_stack_word(&mut buffer, &mut low, *ptr)?;
    }
    write_stack_word(&mut buffer, &mut low, 0)?;
    write_stack_word(&mut buffer, &mut low, 0)?;
    write_stack_word(&mut buffer, &mut low, 0)?;
    write_stack_word(&mut buffer, &mut low, 0)?;

    let stack_pointer = stack_base + words_start as u64;
    let len = USER_INITIAL_STACK_MAX_SIZE - words_start;
    let mut bytes = [0u8; USER_INITIAL_STACK_MAX_SIZE];
    bytes[..len].copy_from_slice(&buffer[words_start..]);

    Ok(UserInitialStack {
        stack_pointer,
        bytes,
        len,
    })
}

pub unsafe fn seed_user_initial_stack(
    layout: UserMemoryLayout,
    program_path: &str,
    arg: &[u8],
) -> Result<u64, UserImageError> {
    let stack = build_user_initial_stack(layout, program_path, arg)?;
    unsafe {
        core::ptr::copy_nonoverlapping(
            stack.as_bytes().as_ptr(),
            stack.stack_pointer as *mut u8,
            stack.len,
        );
    }
    Ok(stack.stack_pointer)
}

fn is_arg_separator(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t')
}

fn write_stack_word(
    buffer: &mut [u8; USER_INITIAL_STACK_MAX_SIZE],
    low: &mut usize,
    value: u64,
) -> Result<(), UserImageError> {
    if *low + 8 > USER_INITIAL_STACK_MAX_SIZE {
        return Err(UserImageError::ImageTooLarge);
    }
    buffer[*low..*low + 8].copy_from_slice(&value.to_le_bytes());
    *low += 8;
    Ok(())
}

pub fn validate_first_user_task_image(image: &[u8]) -> Result<(), UserImageError> {
    if image.is_empty() {
        return Err(UserImageError::EmptyImage);
    }

    if image.len() >= FIRST_USER_DATA_OFFSET {
        return Err(UserImageError::ImageTooLarge);
    }

    Ok(())
}

pub unsafe fn load_first_user_task_image(image: &[u8]) -> Result<(), UserImageError> {
    validate_first_user_task_image(image)?;

    unsafe {
        clear_user_program_area();
        core::ptr::copy_nonoverlapping(image.as_ptr(), FIRST_USER_ENTRY as *mut u8, image.len());
        seed_first_user_data();
    }

    Ok(())
}

pub unsafe fn load_user_elf_image(image: &[u8]) -> Result<(u64, u64), UserImageError> {
    let mut segments = [crate::user::elf::LoadSegment::EMPTY; 8];
    let count = crate::user::elf::load_segments(image, &mut segments)
        .map_err(|_| UserImageError::ImageTooLarge)?;
    let segments = &segments[..count];
    let info = crate::user::elf::parse_elf64(image).map_err(|_| UserImageError::ImageTooLarge)?;

    unsafe {
        crate::user::elf::materialize_load_segments(image, segments)
            .map_err(|_| UserImageError::ImageTooLarge)?;
    }

    Ok((info.entry, crate::user::elf::image_footprint_size(segments)))
}

pub unsafe fn seed_first_user_arg(path: &[u8]) -> Result<(), UserImageError> {
    if path.is_empty() {
        return Err(UserImageError::EmptyImage);
    }

    if path.len() as u64 > FIRST_USER_ARG_PATH_LEN {
        return Err(UserImageError::ImageTooLarge);
    }

    unsafe {
        core::ptr::write_bytes(
            FIRST_USER_ARG_PATH_ADDR as *mut u8,
            0,
            FIRST_USER_ARG_PATH_LEN as usize,
        );
        core::ptr::copy_nonoverlapping(
            path.as_ptr(),
            FIRST_USER_ARG_PATH_ADDR as *mut u8,
            path.len(),
        );
        core::ptr::write(FIRST_USER_ARG_LEN_ADDR as *mut u64, path.len() as u64);
    }

    Ok(())
}

pub unsafe fn seed_first_user_error_message(message: &[u8]) -> Result<(), UserImageError> {
    if message.len() > 16 {
        return Err(UserImageError::ImageTooLarge);
    }

    unsafe {
        core::ptr::write_bytes(FIRST_USER_ERROR_MSG_ADDR as *mut u8, 0, 16);
        core::ptr::copy_nonoverlapping(
            message.as_ptr(),
            FIRST_USER_ERROR_MSG_ADDR as *mut u8,
            message.len(),
        );
    }

    Ok(())
}

unsafe fn seed_first_user_data() {
    unsafe {
        core::ptr::copy_nonoverlapping(
            FIRST_USER_MESSAGE.as_ptr(),
            FIRST_USER_MESSAGE_ADDR as *mut u8,
            FIRST_USER_MESSAGE.len(),
        );
        core::ptr::write_bytes(FIRST_USER_NEWLINE_ADDR as *mut u8, b'\n', 1);
        core::ptr::write_bytes(
            FIRST_USER_UPTIME_BUFFER_ADDR as *mut u8,
            0,
            FIRST_USER_UPTIME_BUFFER_LEN as usize,
        );
        seed_first_user_arg(b"/").ok();
        core::ptr::copy_nonoverlapping(
            FIRST_USER_LS_HEADER.as_ptr(),
            FIRST_USER_LS_HEADER_ADDR as *mut u8,
            FIRST_USER_LS_HEADER.len(),
        );
        core::ptr::write_bytes(FIRST_USER_ROOT_PATH_ADDR as *mut u8, b'/', 1);
        core::ptr::copy_nonoverlapping(
            FIRST_USER_CAT_PATH.as_ptr(),
            FIRST_USER_ROOT_PATH_ADDR as *mut u8,
            FIRST_USER_CAT_PATH.len(),
        );
        core::ptr::write_bytes(
            FIRST_USER_LIST_BUFFER_ADDR as *mut u8,
            0,
            FIRST_USER_LIST_BUFFER_LEN as usize,
        );
        seed_first_user_error_message(FIRST_USER_CAT_ERROR).ok();
    }
}

/// Switches from kernel mode to the first Ring-3 task.
///
/// This is deliberately not called during normal boot yet. The current kernel
/// still needs user code mapping before this can safely become part of boot.
pub unsafe fn jump_to_first_user_task() -> ! {
    unsafe {
        jump_to_user(
            VirtAddr::new(FIRST_USER_ENTRY),
            VirtAddr::new(FIRST_USER_STACK_TOP),
        )
    }
}

pub unsafe fn jump_to_user(entry: VirtAddr, stack_top: VirtAddr) -> ! {
    let selectors = gdt::user_selectors();
    let user_cs = selectors.code.0 as u64;
    let user_ss = selectors.data.0 as u64;
    let rflags = 0x202u64;

    // SAFETY: callers provide mapped Ring-3 entry/stack addresses. The GDT
    // selectors are installed user segments and the constructed iretq frame
    // changes privilege only after all values are placed on the kernel stack.
    unsafe {
        asm!(
            "push {user_ss}",
            "push {user_rsp}",
            "push {rflags}",
            "push {user_cs}",
            "push {user_rip}",
            "iretq",
            user_ss = in(reg) user_ss,
            user_rsp = in(reg) stack_top.as_u64(),
            rflags = in(reg) rflags,
            user_cs = in(reg) user_cs,
            user_rip = in(reg) entry.as_u64(),
            options(noreturn)
        );
    }
}

pub unsafe fn switch_to_user_p4_and_jump(p4_frame: u64, entry: VirtAddr, stack_top: VirtAddr) -> ! {
    let selectors = gdt::user_selectors();
    let user_cs = selectors.code.0 as u64;
    let user_ss = selectors.data.0 as u64;
    let rflags = 0x202u64;

    // SAFETY: p4_frame is a verified prepared user P4 that retains kernel
    // mappings; entry and stack_top belong to that process address space.
    unsafe {
        asm!(
            "mov cr3, {p4_frame}",
            "push {user_ss}",
            "push {user_rsp}",
            "push {rflags}",
            "push {user_cs}",
            "push {user_rip}",
            "iretq",
            p4_frame = in(reg) p4_frame,
            user_ss = in(reg) user_ss,
            user_rsp = in(reg) stack_top.as_u64(),
            rflags = in(reg) rflags,
            user_cs = in(reg) user_cs,
            user_rip = in(reg) entry.as_u64(),
            options(noreturn)
        );
    }
}

pub unsafe fn resume_user(
    context: crate::user::process::UserResumeContext,
    return_value: u64,
) -> ! {
    let selectors = gdt::user_selectors();
    let user_cs = selectors.code.0 as u64;
    let user_ss = selectors.data.0 as u64;
    let rflags = context.rflags | 0x202;
    let context_ptr = &context as *const crate::user::process::UserResumeContext as u64;

    // SAFETY: context was captured from the same blocked/yielded Ring-3 task;
    // its RIP/RSP and register image remain valid until this one-way iretq.
    unsafe {
        asm!(
            "push {user_ss}",
            "push {user_rsp}",
            "push {rflags}",
            "push {user_cs}",
            "push {user_rip}",
            "mov rbx, [r10 + 24]",
            "mov rcx, [r10 + 32]",
            "mov rdx, [r10 + 40]",
            "mov rsi, [r10 + 48]",
            "mov rdi, [r10 + 56]",
            "mov rbp, [r10 + 64]",
            "mov r8, [r10 + 72]",
            "mov r9, [r10 + 80]",
            "mov r11, [r10 + 96]",
            "mov r12, [r10 + 104]",
            "mov r13, [r10 + 112]",
            "mov r14, [r10 + 120]",
            "mov r15, [r10 + 128]",
            "mov r10, [r10 + 88]",
            "iretq",
            in("rax") return_value,
            in("r10") context_ptr,
            user_ss = in(reg) user_ss,
            user_rsp = in(reg) context.rsp,
            rflags = in(reg) rflags,
            user_cs = in(reg) user_cs,
            user_rip = in(reg) context.rip,
            options(noreturn)
        );
    }
}

pub unsafe fn switch_to_user_p4_and_resume(
    p4_frame: u64,
    context: crate::user::process::UserResumeContext,
    return_value: u64,
) -> ! {
    let selectors = gdt::user_selectors();
    let user_cs = selectors.code.0 as u64;
    let user_ss = selectors.data.0 as u64;
    let rflags = context.rflags | 0x202;
    let context_ptr = &context as *const crate::user::process::UserResumeContext as u64;

    // SAFETY: p4_frame and context belong to the same scheduled process. The
    // target P4 preserves kernel mappings while exposing only its user pages.
    unsafe {
        asm!(
            "mov cr3, r11",
            "push {user_ss}",
            "push {user_rsp}",
            "push {rflags}",
            "push {user_cs}",
            "push {user_rip}",
            "mov rbx, [r10 + 24]",
            "mov rcx, [r10 + 32]",
            "mov rdx, [r10 + 40]",
            "mov rsi, [r10 + 48]",
            "mov rdi, [r10 + 56]",
            "mov rbp, [r10 + 64]",
            "mov r8, [r10 + 72]",
            "mov r9, [r10 + 80]",
            "mov r12, [r10 + 104]",
            "mov r13, [r10 + 112]",
            "mov r14, [r10 + 120]",
            "mov r15, [r10 + 128]",
            "mov r11, [r10 + 96]",
            "mov r10, [r10 + 88]",
            "iretq",
            in("rax") return_value,
            in("r10") context_ptr,
            in("r11") p4_frame,
            user_ss = in(reg) user_ss,
            user_rsp = in(reg) context.rsp,
            rflags = in(reg) rflags,
            user_cs = in(reg) user_cs,
            user_rip = in(reg) context.rip,
            options(noreturn)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FIRST_USER_ARG_LEN_ADDR, FIRST_USER_ARG_PATH_ADDR, FIRST_USER_ARG_PATH_LEN,
        FIRST_USER_CAT_ERROR, FIRST_USER_CAT_PATH, FIRST_USER_ENTRY, FIRST_USER_ERROR_MSG_ADDR,
        FIRST_USER_LIST_BUFFER_ADDR, FIRST_USER_LS_ERROR, FIRST_USER_LS_HEADER_ADDR,
        FIRST_USER_MESSAGE, FIRST_USER_MESSAGE_ADDR, FIRST_USER_NEWLINE_ADDR,
        FIRST_USER_ROOT_PATH_ADDR, FIRST_USER_STACK_TOP, FIRST_USER_UPTIME_BUFFER_ADDR,
        USER_INITIAL_STACK_MAX_SIZE, USER_PROGRAM_AREA_SIZE, USER_STACK_REGION_SIZE,
        USER_STACK_REGION_START, UserImageError, build_user_initial_stack, clear_user_stack_slot,
        first_user_task, memory_layout_for_pid, memory_layout_for_pid_with_program_size,
        stack_top_for_pid, user_page_flags, validate_first_user_task_image,
    };
    use x86_64::structures::paging::PageTableFlags;

    #[test_case]
    fn first_user_task_metadata_matches_stub() {
        let task = first_user_task();

        assert!(task.image_len > 0);
        assert_eq!(task.entry, FIRST_USER_ENTRY);
        assert_eq!(task.stack_top, FIRST_USER_STACK_TOP);
    }

    #[test_case]
    fn user_data_addresses_are_fixed_after_code() {
        assert_eq!(FIRST_USER_MESSAGE_ADDR, FIRST_USER_ENTRY + 0x300);
        assert_eq!(FIRST_USER_NEWLINE_ADDR, FIRST_USER_ENTRY + 0x30b);
        assert_eq!(FIRST_USER_UPTIME_BUFFER_ADDR, FIRST_USER_ENTRY + 0x320);
        assert_eq!(FIRST_USER_ARG_PATH_ADDR, FIRST_USER_ENTRY + 0x340);
        assert_eq!(FIRST_USER_ARG_PATH_LEN, 32);
        assert_eq!(FIRST_USER_LS_HEADER_ADDR, FIRST_USER_ENTRY + 0x360);
        assert_eq!(FIRST_USER_ROOT_PATH_ADDR, FIRST_USER_ENTRY + 0x366);
        assert_eq!(FIRST_USER_ARG_LEN_ADDR, FIRST_USER_ENTRY + 0x378);
        assert_eq!(FIRST_USER_LIST_BUFFER_ADDR, FIRST_USER_ENTRY + 0x380);
        assert_eq!(FIRST_USER_ERROR_MSG_ADDR, FIRST_USER_ENTRY + 0x3f0);
        assert_eq!(FIRST_USER_MESSAGE.len(), 11);
        assert_eq!(FIRST_USER_CAT_PATH.len(), 7);
        assert_eq!(FIRST_USER_CAT_ERROR.len(), 15);
        assert_eq!(FIRST_USER_LS_ERROR.len(), 14);
    }

    #[test_case]
    fn user_pages_are_marked_user_accessible() {
        assert!(user_page_flags().contains(PageTableFlags::USER_ACCESSIBLE));
    }

    #[test_case]
    fn user_program_area_stops_before_stack_guard() {
        assert_eq!(USER_PROGRAM_AREA_SIZE, 0x3d8000);
        assert_eq!(
            USER_STACK_REGION_START,
            FIRST_USER_STACK_TOP - USER_STACK_REGION_SIZE
        );
        assert_eq!(USER_STACK_REGION_SIZE, 0x28000);
    }

    #[test_case]
    fn user_stack_slots_are_assigned_from_top_down() {
        assert_eq!(stack_top_for_pid(2), FIRST_USER_STACK_TOP);
        assert_eq!(stack_top_for_pid(3), FIRST_USER_STACK_TOP - 0x5000);
        assert_eq!(stack_top_for_pid(9), USER_STACK_REGION_START + 0x5000);
        assert_eq!(stack_top_for_pid(10), FIRST_USER_STACK_TOP);
    }

    #[test_case]
    fn user_memory_layout_tracks_program_and_stack_ranges() {
        let layout = memory_layout_for_pid(3);

        assert_eq!(layout.program_start, FIRST_USER_ENTRY);
        assert_eq!(layout.program_end, USER_STACK_REGION_START);
        assert_eq!(layout.stack_start, FIRST_USER_STACK_TOP - 0x9000);
        assert_eq!(layout.stack_top, FIRST_USER_STACK_TOP - 0x5000);
    }

    #[test_case]
    fn user_memory_layout_can_track_actual_program_footprint() {
        let layout = memory_layout_for_pid_with_program_size(4, 123);

        assert_eq!(layout.program_start, FIRST_USER_ENTRY);
        assert_eq!(layout.program_end, FIRST_USER_ENTRY + 0x1000);
        assert_eq!(layout.stack_start, FIRST_USER_STACK_TOP - 0xe000);
        assert_eq!(layout.stack_top, FIRST_USER_STACK_TOP - 0xa000);

        let layout = memory_layout_for_pid_with_program_size(4, 0x2100);
        assert_eq!(layout.program_end, FIRST_USER_ENTRY + 0x3000);
    }

    #[test_case]
    fn initial_user_stack_contains_argc_and_argv_pointers() {
        let layout = memory_layout_for_pid_with_program_size(4, 123);
        let stack = build_user_initial_stack(layout, "/bin/cat", b"README MOTD").unwrap();

        assert!(stack.stack_pointer >= layout.stack_top - USER_INITIAL_STACK_MAX_SIZE as u64);
        assert!(stack.stack_pointer < layout.stack_top);
        assert_eq!(stack.stack_pointer % 16, 0);
        assert_eq!(read_u64(stack.as_bytes(), 0), 3);

        let argv0 = read_u64(stack.as_bytes(), 8);
        let argv1 = read_u64(stack.as_bytes(), 16);
        let argv2 = read_u64(stack.as_bytes(), 24);
        let argv_end = read_u64(stack.as_bytes(), 32);

        assert!(argv0 >= stack.stack_pointer && argv0 < layout.stack_top);
        assert!(argv1 >= stack.stack_pointer && argv1 < layout.stack_top);
        assert!(argv2 >= stack.stack_pointer && argv2 < layout.stack_top);
        assert_eq!(argv_end, 0);
    }

    #[test_case]
    fn rejects_stack_clear_outside_user_stack_region() {
        assert_eq!(
            unsafe { clear_user_stack_slot(FIRST_USER_STACK_TOP + 0x1000) },
            Err(UserImageError::ImageTooLarge)
        );
        assert_eq!(
            unsafe { clear_user_stack_slot(USER_STACK_REGION_START) },
            Err(UserImageError::ImageTooLarge)
        );
    }

    #[test_case]
    fn flat_user_image_must_fit_before_data_area() {
        let too_large = [0u8; 0x300];

        assert_eq!(
            validate_first_user_task_image(&[]),
            Err(UserImageError::EmptyImage)
        );
        assert_eq!(
            validate_first_user_task_image(&too_large),
            Err(UserImageError::ImageTooLarge)
        );
        for image in crate::user::images::USERLAND_IMAGES {
            assert_eq!(validate_first_user_task_image(image.data), Ok(()));
        }
    }

    #[test_case]
    fn empty_elf_segment_list_has_zero_memory_size() {
        assert_eq!(crate::user::elf::image_memory_size(&[]), 0);
    }

    fn read_u64(bytes: &[u8], offset: usize) -> u64 {
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes[offset..offset + 8]);
        u64::from_le_bytes(word)
    }
}
