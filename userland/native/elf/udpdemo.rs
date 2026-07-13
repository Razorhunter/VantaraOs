#![no_std]
#![no_main]

#[path = "../src/abi.rs"]
mod abi;

use core::panic::PanicInfo;

const LOCAL_PORT: u16 = 40100;
const PEER_IP: [u8; 4] = [10, 0, 2, 16];
const PEER_PORT: u16 = LOCAL_PORT;
const PAYLOAD: &[u8] = b"USERUDP";

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let handle = abi::udp_bind(LOCAL_PORT);
    if handle < 0 {
        abi::write("udpdemo: bind failed\n");
        abi::exit(1);
    }
    abi::write("udpdemo: bind ok\n");

    if abi::udp_bind(LOCAL_PORT) != abi::ERR_INVALID_ARGUMENT {
        abi::write("udpdemo: duplicate bind semantics failed\n");
        let _ = abi::udp_close(handle as u64);
        abi::exit(2);
    }
    abi::write("udpdemo: duplicate guard ok\n");

    let sent = abi::udp_send_to(handle as u64, PEER_IP, PEER_PORT, PAYLOAD);
    if sent == PAYLOAD.len() as i64 {
        abi::write("udpdemo: send ok\n");
    } else if sent == abi::ERR_WOULD_BLOCK {
        abi::write("udpdemo: send unavailable\n");
    } else {
        abi::write("udpdemo: send failed\n");
        let _ = abi::udp_close(handle as u64);
        abi::exit(3);
    }

    let mut buffer = [0u8; 32];
    let mut meta = abi::UdpDatagramMeta::default();
    if abi::udp_recv_from(handle as u64, &mut buffer, &mut meta) != abi::ERR_WOULD_BLOCK {
        abi::write("udpdemo: empty receive semantics failed\n");
        let _ = abi::udp_close(handle as u64);
        abi::exit(4);
    }
    abi::write("udpdemo: empty receive ok\n");

    if sent == PAYLOAD.len() as i64 {
        // Sleeping returns control to the kernel runtime thread, which polls
        // the peer NIC and dispatches the datagram into this socket queue.
        let _ = abi::sleep_ms(50);
        let received = abi::udp_recv_from(handle as u64, &mut buffer, &mut meta);
        if received != PAYLOAD.len() as i64
            || &buffer[..PAYLOAD.len()] != PAYLOAD
            || meta.source_ip != u32::from_be_bytes([10, 0, 2, 15])
            || meta.source_port != LOCAL_PORT
            || meta.destination_port != LOCAL_PORT
        {
            abi::write("udpdemo: runtime receive failed\n");
            let _ = abi::udp_close(handle as u64);
            abi::exit(5);
        }
        abi::write("udpdemo: runtime receive ok\n");
    }

    if abi::udp_close(handle as u64) < 0 {
        abi::write("udpdemo: close failed\n");
        abi::exit(6);
    }

    abi::write("udpdemo: done\n");
    abi::exit(0);
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::write("udpdemo: panic\n");
    abi::exit(99);
}
