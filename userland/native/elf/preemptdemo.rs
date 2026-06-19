#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

const RUN_FOR_MS: u64 = 2_000;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    abi::write("preemptdemo: start\n");

    let deadline = abi::uptime_ms().saturating_add(RUN_FOR_MS);
    while abi::uptime_ms() < deadline {
        core::hint::spin_loop();
    }

    abi::write("preemptdemo: done\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
