#![no_std]
#![no_main]

use core::{
    panic::PanicInfo,
    sync::atomic::{AtomicU64, Ordering},
};

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

const WORKER_STACK_SIZE: usize = 4096;
static mut WORKER_STACK: [u8; WORKER_STACK_SIZE] = [0; WORKER_STACK_SIZE];
static WORKER_RESULT: AtomicU64 = AtomicU64::new(0);

extern "C" fn worker(arg: u64) -> ! {
    abi::write("threaddemo: worker started\n");
    WORKER_RESULT.store(arg, Ordering::Release);
    abi::write("threaddemo: worker exited\n");
    abi::thread_exit();
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    abi::write("threaddemo: main started\n");

    // SAFETY: only the main thread creates this slice, before the worker can
    // run; the buffer then belongs exclusively to that worker as its stack.
    let stack = unsafe {
        core::slice::from_raw_parts_mut(
            core::ptr::addr_of_mut!(WORKER_STACK).cast::<u8>(),
            WORKER_STACK_SIZE,
        )
    };
    let tid = abi::thread_create(worker, stack, 42);
    if tid < 0 {
        abi::write("threaddemo: thread_create failed\n");
        abi::exit(1);
    }

    abi::write("threaddemo: worker created\n");
    while WORKER_RESULT.load(Ordering::Acquire) != 42 {
        abi::yield_now();
    }

    abi::write("threaddemo: joined\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(2);
}
