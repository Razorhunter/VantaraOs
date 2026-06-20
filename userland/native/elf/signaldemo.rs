#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

extern "C" fn handle_sigterm(signal: u64) -> ! {
    if signal == abi::SIGTERM {
        abi::write("signaldemo: handler received SIGTERM\n");
    } else {
        abi::write("signaldemo: handler received unexpected signal\n");
    }
    abi::sigreturn()
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    if abi::sigaction(abi::SIGTERM, handle_sigterm) < 0 {
        abi::write("signaldemo: sigaction failed\n");
        abi::exit(1);
    }
    abi::write("signaldemo: handler installed\n");

    if abi::sigprocmask(abi::SIG_BLOCK, abi::SIGTERM_MASK) < 0 {
        abi::write("signaldemo: block failed\n");
        abi::exit(1);
    }
    abi::write("signaldemo: SIGTERM blocked\n");

    if abi::sleep_ms(10_000) < 0 {
        abi::write("signaldemo: sleep failed\n");
        abi::exit(1);
    }

    let pending = abi::sigpending();
    if pending < 0 || pending as u64 & abi::SIGTERM_MASK == 0 {
        abi::write("signaldemo: pending SIGTERM missing\n");
        abi::exit(1);
    }
    abi::write("signaldemo: SIGTERM pending\n");
    abi::write("signaldemo: unblocking\n");

    if abi::sigprocmask(abi::SIG_UNBLOCK, abi::SIGTERM_MASK) < 0 {
        abi::write("signaldemo: unblock failed\n");
        abi::exit(1);
    }
    abi::write("signaldemo: resumed after handler\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
