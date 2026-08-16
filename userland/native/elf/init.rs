#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

const LOGIN_PATH: &[u8] = b"/bin/login";
const RESTART_DELAY_MS: u64 = 250;
const DISPLAY_NODES: [(&str, &str); 3] = [
    ("/dev/fb0", "[init-display] fb0 "),
    ("/dev/display0", "[init-display] display0 "),
    ("/dev/virtio-gpu", "[init-display] virtio-gpu "),
];

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    abi::debug("Vantara init started\n");
    abi::debug("[init] service manager running as pid 1\n");
    probe_display_nodes();

    loop {
        abi::debug("[init] starting login\n");
        let login_pid = abi::exec_bytes(LOGIN_PATH, None);
        if login_pid < 0 {
            abi::debug("[init] failed to start login; retrying\n");
            let _ = abi::sleep_ms(RESTART_DELAY_MS);
            continue;
        }

        if abi::waitpid(login_pid as u64) < 0 {
            abi::debug("[init] waitpid failed; retrying login\n");
        } else {
            abi::debug("[init] login exited; restarting\n");
        }
        let _ = abi::sleep_ms(RESTART_DELAY_MS);
    }
}

fn probe_display_nodes() {
    let mut buffer = [0u8; 512];
    for (path, prefix) in DISPLAY_NODES {
        let fd = abi::open(path);
        if fd < 0 {
            abi::debug(prefix);
            abi::debug("unavailable\n");
            continue;
        }
        let count = abi::read(fd as u64, &mut buffer);
        let _ = abi::close(fd as u64);
        abi::debug(prefix);
        if count > 0 {
            abi::write_bytes(abi::STDERR, &buffer[..count as usize]);
            if buffer[count as usize - 1] != b'\n' {
                abi::debug("\n");
            }
        } else {
            abi::debug("read-failed\n");
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::debug("[init] panic; halting service manager\n");
    loop {
        abi::yield_now();
    }
}
