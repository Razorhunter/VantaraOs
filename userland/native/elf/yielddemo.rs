#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    abi::write("yielddemo: start\n");

    let mut i = 0;
    while i < 3 {
        abi::yield_now();
        abi::write("yielddemo: yield\n");
        i += 1;
    }

    abi::write("yielddemo: done\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
