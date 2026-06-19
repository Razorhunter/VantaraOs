#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut buffer = [0u8; 128];
    let len = abi::getcwd(&mut buffer);

    if len < 0 {
        abi::write("pwd: failed\n");
        abi::exit(1);
    }

    abi::write_bytes(abi::STDOUT, &buffer[..len as usize]);
    abi::write("\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
