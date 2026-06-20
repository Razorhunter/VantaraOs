#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut args = abi::argv();
    let Some(arg) = args.next_arg() else {
        abi::write("usage: kill <pid> [signal]\n");
        abi::exit(1);
    };

    let Some(pid) = parse_u64(arg) else {
        abi::write("usage: kill <pid> [signal]\n");
        abi::exit(1);
    };
    let signal = match args.next_arg() {
        Some(value) => match parse_u64(value) {
            Some(signal) => signal,
            None => {
                abi::write("usage: kill <pid> [signal]\n");
                abi::exit(1);
            }
        },
        None => abi::SIGTERM,
    };

    let ret = abi::kill(pid, signal);
    if ret < 0 {
        abi::write("kill: failed\n");
        abi::exit(1);
    }

    if ret == 1 {
        abi::write("kill: signal pending\n");
    } else {
        abi::write("kill: signal delivered\n");
    }
    abi::exit(0);
}

fn parse_u64(bytes: &[u8]) -> Option<u64> {
    let mut value = 0u64;
    let mut saw_digit = false;

    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        saw_digit = true;
        value = value
            .checked_mul(10)?
            .checked_add(u64::from(byte - b'0'))?;
    }

    saw_digit.then_some(value)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
