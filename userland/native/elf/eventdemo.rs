#![no_std]
#![no_main]

#[path = "../src/abi.rs"]
mod abi;

use core::panic::PanicInfo;

extern "C" fn signaler(event: u64) -> ! {
    abi::write("eventdemo: worker signaling\n");
    if abi::event_signal(event) < 0 {
        abi::write("eventdemo: worker signal failed\n");
    }
    abi::thread_exit();
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let event = abi::event_create();
    if event < 0 {
        abi::write("eventdemo: create failed\n");
        abi::exit(1);
    }
    abi::write("eventdemo: created\n");

    let tid = abi::thread_spawn(signaler, event as u64);
    if tid < 0 {
        abi::write("eventdemo: worker create failed\n");
        abi::exit(2);
    }

    abi::write("eventdemo: waiting\n");
    if abi::event_wait(event as u64) < 0 {
        abi::write("eventdemo: wait failed\n");
        abi::exit(3);
    }
    abi::write("eventdemo: woke\n");
    if abi::thread_join(tid as u64) < 0 {
        abi::write("eventdemo: join failed\n");
        abi::exit(4);
    }

    if abi::event_signal(event as u64) < 0 || abi::event_wait(event as u64) < 0 {
        abi::write("eventdemo: latched signal failed\n");
        abi::exit(5);
    }
    abi::write("eventdemo: latched signal ok\n");

    if abi::event_close(event as u64) < 0 {
        abi::write("eventdemo: close failed\n");
        abi::exit(6);
    }
    abi::write("eventdemo: done\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::write("eventdemo: panic\n");
    abi::exit(99);
}
