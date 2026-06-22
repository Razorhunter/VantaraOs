#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

const LOGIN_PATH: &[u8] = b"/bin/login";
const RESTART_DELAY_MS: u64 = 250;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    abi::write("Vantara init started\n");
    abi::write("[init] service manager running as pid 1\n");

    loop {
        abi::write("[init] starting login\n");
        let login_pid = abi::exec_bytes(LOGIN_PATH, None);
        if login_pid < 0 {
            abi::write("[init] failed to start login; retrying\n");
            let _ = abi::sleep_ms(RESTART_DELAY_MS);
            continue;
        }

        if abi::waitpid(login_pid as u64) < 0 {
            abi::write("[init] waitpid failed; retrying login\n");
        } else {
            abi::write("[init] login exited; restarting\n");
        }
        let _ = abi::sleep_ms(RESTART_DELAY_MS);
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::write("[init] panic; halting service manager\n");
    loop {
        abi::yield_now();
    }
}
