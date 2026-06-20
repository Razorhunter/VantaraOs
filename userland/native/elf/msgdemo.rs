#![no_std]
#![no_main]

#[path = "../src/abi.rs"]
mod abi;

use core::panic::PanicInfo;

extern "C" fn sender(queue: u64) -> ! {
    if abi::msgq_send(queue, b"alpha") != 5 || abi::msgq_send(queue, b"beta") != 4 {
        abi::write("msgdemo: worker send failed\n");
    }
    abi::thread_exit();
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let queue = abi::msgq_create();
    if queue < 0 {
        abi::write("msgdemo: create failed\n");
        abi::exit(1);
    }
    abi::write("msgdemo: created\n");

    let mut buffer = [0u8; 16];
    if abi::msgq_receive(queue as u64, &mut buffer) != abi::ERR_WOULD_BLOCK {
        abi::write("msgdemo: empty semantics failed\n");
        abi::exit(2);
    }

    let tid = abi::thread_spawn(sender, queue as u64);
    if tid < 0 {
        abi::write("msgdemo: worker create failed\n");
        abi::exit(3);
    }

    let mut small = [0u8; 2];
    loop {
        let result = abi::msgq_receive(queue as u64, &mut small);
        if result == abi::ERR_WOULD_BLOCK {
            abi::yield_now();
            continue;
        }
        if result != abi::ERR_INVALID_ARGUMENT {
            abi::write("msgdemo: boundary semantics failed\n");
            abi::exit(4);
        }
        break;
    }

    let first = abi::msgq_receive(queue as u64, &mut buffer);
    if first != 5 || &buffer[..5] != b"alpha" {
        abi::write("msgdemo: first message mismatch\n");
        abi::exit(5);
    }
    let second = abi::msgq_receive(queue as u64, &mut buffer);
    if second != 4 || &buffer[..4] != b"beta" {
        abi::write("msgdemo: second message mismatch\n");
        abi::exit(6);
    }
    abi::write("msgdemo: ordered messages ok\n");

    if abi::thread_join(tid as u64) < 0 || abi::msgq_close(queue as u64) < 0 {
        abi::write("msgdemo: cleanup failed\n");
        abi::exit(7);
    }
    abi::write("msgdemo: done\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::write("msgdemo: panic\n");
    abi::exit(99);
}
