#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let Some(arg) = abi::first_arg() else {
        abi::write("usage: sleep <ms>\n");
        abi::exit(1);
    };

    let Some(duration_ms) = parse_u64(arg) else {
        abi::write("usage: sleep <ms>\n");
        abi::exit(1);
    };

    if abi::sleep_ms(duration_ms) < 0 {
        abi::write("sleep: failed\n");
        abi::exit(1);
    }

    abi::exit(0);
}

fn parse_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }

    let mut value = 0u64;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(u64::from(byte - b'0'));
    }

    Some(value)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
