#![no_std]
#![feature(abi_x86_interrupt)]
#![deny(unsafe_op_in_unsafe_fn)]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

#[cfg(test)]
use bootloader::{BootInfo, entry_point};
use core::panic::PanicInfo;

extern crate alloc;

pub mod allocator;
pub mod build_info;
pub mod console;
pub mod diagnostics;
pub mod drivers;
pub mod fs;
pub mod gdt;
pub mod input;
pub mod interrupts;
pub mod memory;
pub mod power;
pub mod runtime;
pub mod scheduler;
pub mod serial;
pub mod shell;
pub mod storage;
pub mod sync;
pub mod timer;
pub mod user;
pub mod vga_buffer;

pub fn init() {
    serial::init();
    diagnostics::print_banner();
    gdt::init();
    diagnostics::mark_gdt();
    interrupts::init_idt();
    diagnostics::mark_idt();
    unsafe {
        interrupts::PICS.lock().initialize();
    }
    diagnostics::mark_pic();
    timer::init();
    diagnostics::mark_timer();
    scheduler::init();
    diagnostics::mark_scheduler();
    match drivers::ps2::init_controller() {
        Ok(()) => {
            drivers::status::report(
                "ps2",
                drivers::status::DriverState::Ready,
                "controller initialized",
            );
            log_info!("PS/2 controller initialized");
        }
        Err(err) => {
            drivers::status::report("ps2", drivers::status::DriverState::Error, err);
            log_warn!("PS/2 controller init skipped: {}", err);
        }
    }
    match drivers::mouse::MOUSE.init() {
        Ok(()) => {
            drivers::status::report(
                "ps2-mouse",
                drivers::status::DriverState::Ready,
                "input enabled",
            );
            log_info!("PS/2 mouse initialized");
        }
        Err(err) => {
            drivers::status::report("ps2-mouse", drivers::status::DriverState::Degraded, err);
            log_warn!("PS/2 mouse init skipped: {}", err);
        }
    }
}

pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

pub fn hlt_loop_once() {
    x86_64::instructions::hlt();
}

#[cfg(test)]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    init();
    test_main();
    hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info)
}

#[cfg(test)]
entry_point!(test_kernel_main);

#[cfg(test)]
fn test_kernel_main(_boot_info: &'static BootInfo) -> ! {
    init();
    test_main();
    hlt_loop();
}

pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    diagnostics::panic_screen(info);
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    hlt_loop();
}
