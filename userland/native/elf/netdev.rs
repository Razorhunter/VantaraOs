#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut buffer = [0u8; 1024];
    let ret = abi::netdev_list(&mut buffer);

    if ret == 0 {
        abi::write("netdev: failed\n");
        abi::exit(1);
    }

    abi::write_bytes(abi::STDOUT, &buffer[..ret.min(buffer.len())]);
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
