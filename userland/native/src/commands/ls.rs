use crate::abi;

pub fn main() -> ! {
    list("/");
    abi::exit(0);
}

pub fn list(path: &str) {
    let mut buffer = [0u8; 256];
    let len = abi::listdir(path, &mut buffer);

    if len == 0 {
        abi::write("ls: not found or empty\n");
        return;
    }

    abi::write_bytes(abi::STDOUT, &buffer[..len]);
}
