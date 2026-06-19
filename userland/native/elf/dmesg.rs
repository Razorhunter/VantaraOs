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
    let ret = abi::kernel_log(&mut buffer);

    if ret == 0 {
        abi::write("dmesg: log empty\n");
        abi::exit(0);
    }

    abi::write_bytes(abi::STDOUT, &buffer[..ret.min(buffer.len())]);
    if buffer[ret.min(buffer.len()) - 1] != b'\n' {
        abi::write("\n");
    }
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
