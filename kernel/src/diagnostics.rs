use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::{log_error, log_info, println, serial_println};

const CHECK_GDT: u32 = 1 << 0;
const CHECK_IDT: u32 = 1 << 1;
const CHECK_PIC: u32 = 1 << 2;
const CHECK_TIMER: u32 = 1 << 3;
const CHECK_SCHEDULER: u32 = 1 << 4;
const CHECK_INTERRUPTS: u32 = 1 << 5;
const CHECK_USB: u32 = 1 << 6;
const CHECK_HEAP: u32 = 1 << 7;
const CHECK_FRAME_ALLOCATOR: u32 = 1 << 8;

static BOOT_CHECKS: AtomicU32 = AtomicU32::new(0);

pub fn print_banner() {
    serial_println!("");
    serial_println!("========================================");
    serial_println!(" Vantara OS Kernel");
    serial_println!(" version: {}", crate::build_info::VERSION);
    serial_println!(" arch: x86_64");
    serial_println!(" mode: {}", crate::build_info::PROFILE);
    serial_println!(
        " git: {} dirty={}",
        crate::build_info::GIT_COMMIT,
        crate::build_info::GIT_DIRTY
    );
    serial_println!(" built: {}", crate::build_info::BUILD_TIMESTAMP);
    serial_println!("========================================");
}

fn mark(bit: u32, name: &str) {
    BOOT_CHECKS.fetch_or(bit, Ordering::Relaxed);
    log_info!("boot check ok: {}", name);
}

fn mark_quiet(bit: u32, name: &str) {
    BOOT_CHECKS.fetch_or(bit, Ordering::Relaxed);
    log_info!("boot check ok: {}", name);
}

pub fn mark_gdt() {
    mark(CHECK_GDT, "GDT");
}

pub fn mark_idt() {
    mark(CHECK_IDT, "IDT");
}

pub fn mark_pic() {
    mark(CHECK_PIC, "PIC");
}

pub fn mark_timer() {
    mark(CHECK_TIMER, "timer");
}

pub fn mark_scheduler() {
    mark(CHECK_SCHEDULER, "scheduler");
}

pub fn mark_interrupts() {
    mark(CHECK_INTERRUPTS, "interrupts enabled");
}

pub fn mark_usb() {
    mark_quiet(CHECK_USB, "USB init attempted");
}

pub fn mark_heap() {
    mark(CHECK_HEAP, "heap");
}

pub fn mark_frame_allocator() {
    mark(CHECK_FRAME_ALLOCATOR, "frame allocator");
}

pub fn checks() -> u32 {
    BOOT_CHECKS.load(Ordering::Relaxed)
}

pub fn print_summary() {
    let checks = checks();

    serial_println!("");
    serial_println!("Boot self-check summary:");
    print_check(checks, CHECK_GDT, "GDT");
    print_check(checks, CHECK_IDT, "IDT");
    print_check(checks, CHECK_PIC, "PIC");
    print_check(checks, CHECK_TIMER, "timer");
    print_check(checks, CHECK_SCHEDULER, "scheduler");
    print_check(checks, CHECK_INTERRUPTS, "interrupts");
    print_check(checks, CHECK_USB, "USB");
    print_check(checks, CHECK_HEAP, "heap");
    print_check(checks, CHECK_FRAME_ALLOCATOR, "frame allocator");
}

fn print_check(checks: u32, bit: u32, name: &str) {
    if checks & bit != 0 {
        serial_println!("  [ ok ] {}", name);
    } else {
        serial_println!("  [miss] {}", name);
    }
}

pub fn panic_screen(info: &PanicInfo) {
    log_error!("kernel panic: {}", info);
    println!("");
    println!("*** VANTARA OS KERNEL PANIC ***");
    println!("{}", info);
}
