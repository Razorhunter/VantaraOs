use crate::abi;

pub fn main() -> ! {
    cat("/README");
    abi::exit(0);
}

pub fn cat(path: &str) {
    let mut buffer = [0u8; 512];
    let len = abi::read_file(path, &mut buffer);

    if len == 0 {
        abi::write("cat: not found or empty\n");
        return;
    }

    abi::write_bytes(abi::STDOUT, &buffer[..len]);
    if buffer[len - 1] != b'\n' {
        abi::write("\n");
    }
}
