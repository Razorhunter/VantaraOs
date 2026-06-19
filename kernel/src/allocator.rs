use alloc::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicUsize, Ordering};
// use bump::BumpAllocator;
// use linked_list::LinkedListAllocator;
use fixed_size_block::FixedSizeBlockAllocator;
use x86_64::{
    VirtAddr,
    structures::paging::{
        FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB, mapper::MapToError,
    },
};

pub mod bump;
pub mod fixed_size_block;
pub mod linked_list;

pub struct Dummy;

#[global_allocator]
// static ALLOCATOR: Locked<BumpAllocator> = Locked::new(BumpAllocator::new());
// static ALLOCATOR: Locked<LinkedListAllocator> = Locked::new(LinkedListAllocator::new());
static ALLOCATOR: Locked<FixedSizeBlockAllocator> = Locked::new(FixedSizeBlockAllocator::new());

pub const HEAP_START: usize = 0x_4444_4444_0000;
pub const HEAP_SIZE: usize = 1024 * 1024; // 1 MiB

static LIVE_ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_FAILURES: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapStats {
    pub start: usize,
    pub end: usize,
    pub size: usize,
    pub used_bytes: usize,
    pub free_bytes: usize,
    pub allocation_failures: usize,
}

unsafe impl GlobalAlloc for Dummy {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        null_mut()
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        panic!("dealloc should be never called")
    }
}

pub fn init_heap(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), MapToError<Size4KiB>> {
    let page_range = {
        let heap_start = VirtAddr::new(HEAP_START as u64);
        let heap_end = heap_start + HEAP_SIZE as u64 - 1u64;
        let heap_start_page = Page::containing_address(heap_start);
        let heap_end_page = Page::containing_address(heap_end);
        Page::range_inclusive(heap_start_page, heap_end_page)
    };

    for page in page_range {
        let frame = frame_allocator
            .allocate_frame()
            .ok_or(MapToError::FrameAllocationFailed)?;
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        // SAFETY: each page in the dedicated heap range is mapped exactly once
        // to a uniquely allocated frame before the allocator is initialized.
        unsafe { mapper.map_to(page, frame, flags, frame_allocator)?.flush() };
    }

    // SAFETY: the complete HEAP_START..HEAP_START+HEAP_SIZE range is now
    // present, writable, and exclusively owned by the global allocator.
    unsafe {
        ALLOCATOR.lock().init(HEAP_START, HEAP_SIZE);
    }

    Ok(())
}

pub fn heap_stats() -> HeapStats {
    let used_bytes = LIVE_ALLOCATED_BYTES.load(Ordering::Relaxed).min(HEAP_SIZE);

    HeapStats {
        start: HEAP_START,
        end: HEAP_START + HEAP_SIZE,
        size: HEAP_SIZE,
        used_bytes,
        free_bytes: HEAP_SIZE.saturating_sub(used_bytes),
        allocation_failures: ALLOCATION_FAILURES.load(Ordering::Relaxed),
    }
}

pub(super) fn record_alloc(layout: Layout, ptr: *mut u8) {
    if ptr.is_null() {
        ALLOCATION_FAILURES.fetch_add(1, Ordering::Relaxed);
    } else {
        LIVE_ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
    }
}

pub(super) fn record_dealloc(layout: Layout) {
    LIVE_ALLOCATED_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
}

pub struct Locked<A> {
    inner: spin::Mutex<A>,
}

impl<A> Locked<A> {
    pub const fn new(inner: A) -> Self {
        Locked {
            inner: spin::Mutex::new(inner),
        }
    }

    pub fn lock(&self) -> spin::MutexGuard<'_, A> {
        self.inner.lock()
    }
}

fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}
