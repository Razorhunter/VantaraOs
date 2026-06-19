#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    loop {
        abi::write("login: ");

        let mut username = [0u8; 32];
        let len = read_line(&mut username);
        abi::write("\n");

        let username = trim(&username[..len]);
        if username.is_empty() {
            abi::write("login: empty username\n");
            continue;
        }

        if abi::setuser(username) < 0 {
            abi::write("login: session failed\n");
            continue;
        }

        abi::write("Welcome to Vantara OS, ");
        abi::write_bytes(abi::STDOUT, username);
        abi::write("\n");

        let shell_pid = abi::exec_bytes(b"sh", None);
        if shell_pid < 0 {
            abi::write("login: exec /bin/sh failed\n");
            continue;
        }

        if abi::waitpid(shell_pid as u64) < 0 {
            abi::write("login: shell wait failed\n");
        }
    }
}

fn read_line(input: &mut [u8]) -> usize {
    let mut len = 0usize;
    loop {
        let mut byte = [0u8; 1];
        let read = abi::read(abi::STDIN, &mut byte);
        if read == abi::ERR_WOULD_BLOCK || read == 0 {
            abi::yield_now();
            continue;
        } else if read < 0 {
            continue;
        }

        match byte[0] {
            b'\n' => return len,
            8 => {
                if len > 0 {
                    len -= 1;
                    abi::write_bytes(abi::STDOUT, &[8, b' ', 8]);
                }
            }
            ch if (0x20..=0x7e).contains(&ch) => {
                if len < input.len() {
                    input[len] = ch;
                    len += 1;
                    abi::write_bytes(abi::STDOUT, &[ch]);
                }
            }
            _ => {}
        }
    }
}

fn trim(bytes: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = bytes.len();

    while start < end && bytes[start] == b' ' {
        start += 1;
    }
    while end > start && bytes[end - 1] == b' ' {
        end -= 1;
    }

    &bytes[start..end]
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
