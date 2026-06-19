use x86_64::instructions::port::Port;

use crate::{QemuExitCode, exit_qemu, hlt_loop, log_info};

pub fn qemu_shutdown_success() -> ! {
    log_info!("requesting QEMU shutdown: success");
    exit_qemu(QemuExitCode::Success);
    hlt_loop();
}

pub fn qemu_shutdown_failure() -> ! {
    log_info!("requesting QEMU shutdown: failure");
    exit_qemu(QemuExitCode::Failed);
    hlt_loop();
}

pub fn reboot() -> ! {
    log_info!("requesting keyboard-controller CPU reset");
    wait_for_keyboard_controller();

    unsafe {
        let mut command_port = Port::<u8>::new(0x64);
        command_port.write(0xfe);
    }

    hlt_loop();
}

fn wait_for_keyboard_controller() {
    unsafe {
        let mut command_port = Port::<u8>::new(0x64);

        while command_port.read() & 0x02 != 0 {
            core::hint::spin_loop();
        }
    }
}
