#![no_std]
#![no_main]

#[path = "../src/abi.rs"]
mod abi;

use core::panic::PanicInfo;

const PAYLOAD: &[u8] = b"TCPUSR";

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let listener = abi::tcp_listen(8081);
    if listener < 0 {
        fail("tcpdemo: listen failed\n", 1);
    }
    abi::write("tcpdemo: listen ok\n");
    if abi::tcp_accept(listener as u64) != abi::ERR_WOULD_BLOCK {
        fail("tcpdemo: accept semantics failed\n", 2);
    }

    let client = abi::tcp_connect([10, 0, 2, 16], 8081, 40200);
    if client < 0 {
        fail("tcpdemo: connect failed\n", 3);
    }
    let _ = abi::sleep_ms(50);
    let server = abi::tcp_accept(listener as u64);
    if server < 0 {
        fail("tcpdemo: accept failed\n", 4);
    }
    abi::write("tcpdemo: connected\n");

    if abi::tcp_send(client as u64, PAYLOAD) != PAYLOAD.len() as i64 {
        fail("tcpdemo: send failed\n", 5);
    }
    let _ = abi::sleep_ms(50);
    let mut buffer = [0u8; 16];
    if abi::tcp_receive(server as u64, &mut buffer) != PAYLOAD.len() as i64
        || &buffer[..PAYLOAD.len()] != PAYLOAD
    {
        fail("tcpdemo: receive failed\n", 6);
    }
    abi::write("tcpdemo: transfer ok\n");

    let _ = abi::tcp_close(client as u64);
    let _ = abi::tcp_close(server as u64);
    let _ = abi::tcp_close(listener as u64);
    abi::write("tcpdemo: done\n");
    abi::exit(0);
}

fn fail(message: &str, status: u64) -> ! {
    abi::write(message);
    abi::exit(status)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::write("tcpdemo: panic\n");
    abi::exit(99)
}
