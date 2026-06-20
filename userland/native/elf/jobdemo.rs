#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let inherited_group = abi::getpgrp();
    let inherited_session = abi::getsid(0);
    if inherited_group <= 0 || inherited_session <= 0 {
        abi::write("jobdemo: inherited identity failed\n");
        abi::exit(1);
    }
    abi::write("jobdemo: inherited group/session ok\n");

    let session = abi::setsid();
    if session <= 0 {
        abi::write("jobdemo: setsid failed\n");
        abi::exit(1);
    }
    if abi::getpgrp() != session || abi::getsid(0) != session {
        abi::write("jobdemo: session identity mismatch\n");
        abi::exit(1);
    }
    abi::write("jobdemo: new session and group ok\n");

    if abi::setpgid(0, 0) >= 0 {
        abi::write("jobdemo: session leader setpgid accepted\n");
        abi::exit(1);
    }
    abi::write("jobdemo: session leader guard ok\n");
    abi::write("jobdemo: done\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
