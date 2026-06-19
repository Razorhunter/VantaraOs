#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

use alloc::{boxed::Box, rc::Rc, vec, vec::Vec};
use bootloader::{BootInfo, entry_point};
use core::panic::PanicInfo;
use kernel::serial_println;

extern crate alloc;

// Test task entry points
extern "C" fn task_a() -> ! {
    loop {
        serial_println!("[Task A] Running");
        for _ in 0..100 {
            core::hint::spin_loop();
        }
    }
}

extern "C" fn task_b() -> ! {
    loop {
        serial_println!("[Task B] Running");
        for _ in 0..100 {
            core::hint::spin_loop();
        }
    }
}

extern "C" fn task_c() -> ! {
    loop {
        serial_println!("[Task C] Running");
        for _ in 0..100 {
            core::hint::spin_loop();
        }
    }
}

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    use kernel::allocator;
    use kernel::memory;
    use kernel::memory::BootInfoFrameAllocator;
    use x86_64::VirtAddr;

    kernel::init();
    memory::enable_no_execute();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe { BootInfoFrameAllocator::init(&boot_info.memory_map) };

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
    kernel::scheduler::SCHEDULER.create_idle_task();
    kernel::user::init();
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

    // Create test tasks using the scheduler
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
    kernel::shell::init();

    kernel::drivers::pci::init();
    kernel::drivers::network::init();
    kernel::drivers::usb_host::init_usb();
    kernel::diagnostics::mark_usb();
    kernel::fs::init();
    match kernel::user::program::request_path("init") {
        Ok(pid) => serial_println!("[USER] boot policy queued /bin/init pid={}", pid),
        Err(err) => serial_println!("[USER] boot policy skipped /bin/init: {:?}", err),
    }

    // Draw initial mouse cursor and update on input events
    kernel::vga_buffer::draw_mouse_cursor(40, 12);

    kernel::runtime::run_event_loop();
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
