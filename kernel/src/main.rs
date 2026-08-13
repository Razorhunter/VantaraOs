#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

use alloc::{boxed::Box, rc::Rc, vec, vec::Vec};
#[cfg(not(feature = "modern-boot"))]
use bootloader::{BootInfo, entry_point};
#[cfg(feature = "modern-boot")]
use bootloader_api::{BootInfo, entry_point};
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use kernel::serial_println;

extern crate alloc;

static TASK_A_PROGRESS: AtomicU64 = AtomicU64::new(0);
static TASK_B_PROGRESS: AtomicU64 = AtomicU64::new(0);
static RUN_KERNEL_THREAD_TEST: AtomicBool =
    AtomicBool::new(cfg!(feature = "kernel-thread-preemption-test"));

extern "C" fn task_a() -> ! {
    kernel::interrupts::without_interrupts(|| serial_println!("[KTHREAD] A started"));
    {
        let _guard = kernel::sync::PreemptionGuard::new();
        let b_before_guard = TASK_B_PROGRESS.load(Ordering::Relaxed);
        let start_tick = kernel::timer::ticks();
        while kernel::timer::ticks().saturating_sub(start_tick) < 3 {
            TASK_A_PROGRESS.fetch_add(1, Ordering::Relaxed);
            core::hint::spin_loop();
        }
        assert_eq!(
            TASK_B_PROGRESS.load(Ordering::Relaxed),
            b_before_guard,
            "kernel thread switched while preemption was disabled"
        );
    }
    kernel::interrupts::without_interrupts(|| {
        serial_println!("[KTHREAD] preemption guard passed");
    });
    loop {
        TASK_A_PROGRESS.fetch_add(1, Ordering::Relaxed);
        core::hint::spin_loop();
    }
}

extern "C" fn task_b() -> ! {
    kernel::interrupts::without_interrupts(|| serial_println!("[KTHREAD] B started"));
    loop {
        TASK_B_PROGRESS.fetch_add(1, Ordering::Relaxed);
        core::hint::spin_loop();
    }
}

extern "C" fn task_c() -> ! {
    kernel::interrupts::without_interrupts(|| serial_println!("[KTHREAD] C started"));
    let a_before = TASK_A_PROGRESS.load(Ordering::Relaxed);
    let b_before = TASK_B_PROGRESS.load(Ordering::Relaxed);
    while TASK_A_PROGRESS.load(Ordering::Relaxed) <= a_before
        || TASK_B_PROGRESS.load(Ordering::Relaxed) <= b_before
    {
        core::hint::spin_loop();
    }
    kernel::interrupts::without_interrupts(|| {
        serial_println!("[KTHREAD] resume cycle passed");
    });
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(feature = "modern-boot"))]
entry_point!(legacy_kernel_main);

#[cfg(feature = "modern-boot")]
use bootloader_api::config::{BootloaderConfig, Mapping};

#[cfg(feature = "modern-boot")]
static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

#[cfg(feature = "modern-boot")]
entry_point!(modern_kernel_main, config = &BOOTLOADER_CONFIG);

#[derive(Debug, Clone, Copy)]
struct BootFramebuffer {
    address: usize,
    width: usize,
    height: usize,
    stride: usize,
    bits_per_pixel: u8,
    pixel_format: kernel::drivers::framebuffer::PixelFormat,
}

#[cfg(not(feature = "modern-boot"))]
fn legacy_kernel_main(boot_info: &'static BootInfo) -> ! {
    let phys_mem_offset = boot_info.physical_memory_offset;
    let frame_allocator =
        unsafe { kernel::memory::BootInfoFrameAllocator::init(&boot_info.memory_map) };
    kernel_main(phys_mem_offset, frame_allocator, None)
}

#[cfg(feature = "modern-boot")]
fn modern_kernel_main(boot_info: &'static mut BootInfo) -> ! {
    use bootloader_api::info::{Optional, PixelFormat as BootPixelFormat};

    let phys_mem_offset = match boot_info.physical_memory_offset {
        Optional::Some(offset) => offset,
        Optional::None => panic!("bootloader did not map physical memory"),
    };
    let framebuffer = match &mut boot_info.framebuffer {
        Optional::Some(framebuffer) => {
            let info = framebuffer.info();
            let pixel_format = match info.pixel_format {
                BootPixelFormat::U8 => kernel::drivers::framebuffer::PixelFormat::Indexed8,
                BootPixelFormat::Rgb => {
                    if info.bytes_per_pixel == 4 {
                        kernel::drivers::framebuffer::PixelFormat::Bgrx8888
                    } else {
                        kernel::drivers::framebuffer::PixelFormat::Rgb888
                    }
                }
                BootPixelFormat::Bgr => {
                    if info.bytes_per_pixel == 4 {
                        kernel::drivers::framebuffer::PixelFormat::Xrgb8888
                    } else {
                        kernel::drivers::framebuffer::PixelFormat::Bgr888
                    }
                }
                BootPixelFormat::Unknown { .. } => {
                    panic!("unsupported boot framebuffer pixel format")
                }
                _ => panic!("unknown future boot framebuffer pixel format"),
            };
            Some(BootFramebuffer {
                address: framebuffer.buffer_mut().as_mut_ptr() as usize,
                width: info.width,
                height: info.height,
                stride: info.stride.saturating_mul(info.bytes_per_pixel),
                bits_per_pixel: (info.bytes_per_pixel.saturating_mul(8)) as u8,
                pixel_format,
            })
        }
        Optional::None => None,
    };
    let frame_allocator =
        unsafe { kernel::memory::BootInfoFrameAllocator::init(&boot_info.memory_regions) };
    kernel_main(phys_mem_offset, frame_allocator, framebuffer)
}

fn kernel_main(
    physical_memory_offset: u64,
    mut frame_allocator: kernel::memory::BootInfoFrameAllocator,
    boot_framebuffer: Option<BootFramebuffer>,
) -> ! {
    use kernel::allocator;
    use kernel::memory;
    use x86_64::VirtAddr;

    kernel::init();
    memory::enable_no_execute();

    let phys_mem_offset = VirtAddr::new(physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };

    if frame_allocator.has_usable_frames() {
        kernel::diagnostics::mark_frame_allocator();
    }
    let frame_stats = frame_allocator.stats();
    serial_println!(
        "frame allocator: total={} allocated={} remaining={}",
        frame_stats.total_usable,
        frame_stats.allocated,
        frame_stats.remaining
    );

    allocator::init_heap(&mut mapper, &mut frame_allocator).expect("heap initialization failed");
    kernel::diagnostics::mark_heap();
    if let Some(framebuffer) = boot_framebuffer {
        kernel::drivers::framebuffer::init_boot_framebuffer(
            framebuffer.address,
            framebuffer.width,
            framebuffer.height,
            framebuffer.stride,
            framebuffer.bits_per_pixel,
            framebuffer.pixel_format,
        );
    } else {
        kernel::drivers::framebuffer::init();
    }
    kernel::drivers::display::init();
    kernel::drivers::compositor::init();
    kernel::user::init();
    kernel::scheduler::SCHEDULER.create_idle_task();
    kernel::drivers::pci::init();
    kernel::drivers::ahci::init(&mut mapper, &mut frame_allocator, phys_mem_offset);
    kernel::drivers::nvme::init(&mut mapper, &mut frame_allocator, phys_mem_offset);
    kernel::drivers::network::init(&mut mapper, &mut frame_allocator, phys_mem_offset);
    kernel::drivers::ehci::init(&mut mapper, &mut frame_allocator, phys_mem_offset);
    kernel::drivers::xhci::init(&mut mapper, &mut frame_allocator, phys_mem_offset);
    match kernel::user::ring3::map_first_user_task(&mut mapper, &mut frame_allocator) {
        Ok(()) => serial_println!("[USER] first Ring-3 task image and stack mapped"),
        Err(err) => serial_println!("[USER] first Ring-3 task mapping skipped: {:?}", err),
    }
    let prepared_user_spaces =
        kernel::user::address_space::init_private_p4_pool(phys_mem_offset, &mut frame_allocator);
    serial_println!(
        "[USER] prepared private P4 frame pool: count={} active_p4_snapshot=true",
        prepared_user_spaces
    );
    match unsafe { kernel::user::address_space::smoke_switch_to_prepared_p4() } {
        Ok(frame) => serial_println!("[USER] CR3 smoke switch ok: p4={:#x}", frame),
        Err(err) => serial_println!("[USER] CR3 smoke switch skipped: {:?}", err),
    }
    kernel::fs::init();
    kernel::interrupts::enable();
    kernel::diagnostics::mark_interrupts();
    kernel::diagnostics::print_summary();

    let heap_value = Box::new(41);
    serial_println!("heap_value at {:p}", heap_value);

    let mut vec = Vec::new();
    for i in 0..500 {
        vec.push(i);
    }
    serial_println!("vec at {:p}", vec.as_slice());

    let reference_counted = Rc::new(vec![1, 2, 3]);
    let cloned_reference = reference_counted.clone();
    serial_println!(
        "current reference count is {}",
        Rc::strong_count(&cloned_reference)
    );
    core::mem::drop(reference_counted);
    serial_println!(
        "reference count is {} now",
        Rc::strong_count(&cloned_reference)
    );
    let heap_stats = allocator::heap_stats();
    serial_println!(
        "heap: start={:#x} end={:#x} used={} free={} failures={}",
        heap_stats.start,
        heap_stats.end,
        heap_stats.used_bytes,
        heap_stats.free_bytes,
        heap_stats.allocation_failures
    );

    #[cfg(test)]
    test_main();

    serial_println!("Vantara Kernel is running!");

    if RUN_KERNEL_THREAD_TEST.load(Ordering::Relaxed) {
        serial_println!("\n[MAIN] Creating test tasks...");
        let task1_id = kernel::scheduler::SCHEDULER.create_task(task_a);
        let task2_id = kernel::scheduler::SCHEDULER.create_task(task_b);
        let task3_id = kernel::scheduler::SCHEDULER.create_task(task_c);
        kernel::println!("[ok] demo tasks");
        serial_println!("[MAIN] Task IDs: {} {} {}", task1_id, task2_id, task3_id);

        let (task_count, ready_count) = kernel::scheduler::SCHEDULER.get_stats();
        serial_println!(
            "[MAIN] Scheduler stats - Total tasks: {}, Ready tasks: {}",
            task_count,
            ready_count
        );
        serial_println!("[KTHREAD] starting timer-driven Ring-0 switch test");
        // SAFETY: all test threads own initialized kernel stacks and synthetic
        // interrupt frames; this one-way bootstrap intentionally hands control
        // to the timer-driven kernel scheduler.
        unsafe {
            kernel::scheduler::SCHEDULER.start_first_task();
        }
    }

    kernel::shell::init();

    #[cfg(feature = "ahci-write-test")]
    kernel::drivers::ahci::run_write_test();
    #[cfg(feature = "nvme-write-test")]
    kernel::drivers::nvme::run_write_test();
    #[cfg(feature = "ahci-vantfs-test")]
    kernel::fs::run_ahci_vantfs_test();
    kernel::drivers::usb_host::init_usb(&mut frame_allocator, phys_mem_offset);
    #[cfg(feature = "usb-write-test")]
    kernel::drivers::usb_host::run_write_test();
    kernel::diagnostics::mark_usb();
    match kernel::user::program::request_boot_init() {
        Ok(pid) => serial_println!("[USER] boot policy queued /bin/init pid={}", pid),
        Err(err) => serial_println!("[USER] boot policy skipped /bin/init: {:?}", err),
    }

    // Draw initial mouse cursor and update on input events
    kernel::vga_buffer::draw_mouse_cursor(40, 12);

    let runtime_tid =
        kernel::scheduler::SCHEDULER.create_task(kernel::runtime::kernel_event_thread);
    serial_println!(
        "[KTHREAD] normal scheduler starting runtime tid={}",
        runtime_tid
    );
    // SAFETY: boot initialization is complete, the runtime thread owns its
    // initialized kernel stack, and control intentionally never returns to the
    // temporary boot stack.
    unsafe {
        kernel::scheduler::SCHEDULER.start_first_task();
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kernel::diagnostics::panic_screen(info);
    kernel::hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kernel::test_panic_handler(info)
}
