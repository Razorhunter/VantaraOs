#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

extern "C" fn worker(arg: u64) -> ! {
    abi::write("threaddemo: worker started\n");
    let mut uptime = [0u8; 32];
    if abi::uptime_to_buffer(&mut uptime) > 0 {
        abi::write("threaddemo: worker stack buffer ok\n");
    }
    if arg == 42 {
        abi::write("threaddemo: worker argument ok\n");
    }
    abi::write("threaddemo: worker exited\n");
    abi::thread_exit();
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    abi::write("threaddemo: main started\n");

    let tid = abi::thread_spawn(worker, 42);
    if tid < 0 {
        abi::write("threaddemo: thread_create failed\n");
        abi::exit(1);
    }

    abi::write("threaddemo: worker created\n");
    if abi::thread_join(tid as u64) < 0 {
        abi::write("threaddemo: thread_join failed\n");
        abi::exit(2);
    }

    abi::write("threaddemo: joined\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(3);
}
