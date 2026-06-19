use crate::abi;

pub fn main() -> ! {
    let mut buffer = [0u8; 32];
    let len = abi::uptime_to_buffer(&mut buffer);

    abi::write("uptime ms: ");
    abi::write_bytes(abi::STDOUT, &buffer[..len]);
    abi::write("\n");
    abi::exit(0);
}
