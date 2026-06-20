#![no_std]
#![no_main]

#[path = "../src/abi.rs"]
mod abi;

use core::panic::PanicInfo;

const MESSAGE: &[u8] = b"hello through pipe";

extern "C" fn writer(write_fd: u64) -> ! {
    if abi::write_bytes(write_fd, MESSAGE) as i64 != MESSAGE.len() as i64 {
        abi::write("pipedemo: worker write failed\n");
        abi::thread_exit();
    }
    let _ = abi::close(write_fd);
    abi::thread_exit();
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut fds = [0u64; 2];
    if abi::pipe(&mut fds) < 0 {
        abi::write("pipedemo: pipe failed\n");
        abi::exit(1);
    }
    abi::write("pipedemo: created\n");

    let mut empty = [0u8; 8];
    if abi::read(fds[0], &mut empty) != abi::ERR_WOULD_BLOCK {
        abi::write("pipedemo: empty read semantics failed\n");
        abi::exit(2);
    }

    let writer_tid = abi::thread_spawn(writer, fds[1]);
    if writer_tid < 0 {
        abi::write("pipedemo: worker create failed\n");
        abi::exit(3);
    }

    let mut received = [0u8; 32];
    let count = loop {
        let count = abi::read(fds[0], &mut received);
        if count == abi::ERR_WOULD_BLOCK {
            abi::yield_now();
            continue;
        }
        break count;
    };
    if count != MESSAGE.len() as i64 || &received[..MESSAGE.len()] != MESSAGE {
        abi::write("pipedemo: data mismatch\n");
        abi::exit(4);
    }
    abi::write("pipedemo: transfer ok\n");

    if abi::thread_join(writer_tid as u64) < 0 || abi::read(fds[0], &mut received) != 0 {
        abi::write("pipedemo: eof semantics failed\n");
        abi::exit(5);
    }
    if abi::close(fds[0]) < 0 {
        abi::write("pipedemo: close failed\n");
        abi::exit(6);
    }

    abi::write("pipedemo: done\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::write("pipedemo: panic\n");
    abi::exit(99);
}
