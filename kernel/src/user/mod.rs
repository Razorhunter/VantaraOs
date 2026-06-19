pub mod address_space;
pub mod elf;
pub mod images;
pub mod loader;
pub mod process;
pub mod program;
pub mod ring3;
pub mod syscall;
pub mod thread;

pub fn init() {
    process::init_process_table();
    syscall::init();
    crate::serial_println!("[USER] User-mode preparation initialized");
}
