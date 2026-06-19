#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let Some(arg) = abi::first_arg() else {
        abi::write("usage: stat <path>\n");
        abi::exit(1);
    };

    let mut stat = abi::FileStat {
        size: 0,
        readonly: 0,
        file_type: 0,
        inode: 0,
    };
    let ret = abi::file_stat(arg, &mut stat);

    if ret < 0 {
        abi::write("stat: not found\n");
        abi::exit(1);
    }

    abi::write("path: ");
    abi::write_bytes(abi::STDOUT, arg);
    abi::write("\nsize: ");
    write_u64(stat.size);
    abi::write("\nreadonly: ");
    abi::write(if stat.readonly == 0 { "0" } else { "1" });
    abi::write("\ntype: ");
    abi::write(match stat.file_type {
        1 => "file",
        2 => "dir",
        _ => "unknown",
    });
    abi::write("\ninode: ");
    write_u64(stat.inode);
    abi::write("\n");
    abi::exit(0);
}

fn write_u64(mut value: u64) {
    let mut buffer = [0u8; 20];
    if value == 0 {
        abi::write("0");
        return;
    }

    let mut len = 0usize;
    while value > 0 {
        buffer[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }

    while len > 0 {
        len -= 1;
        abi::write_bytes(abi::STDOUT, &buffer[len..len + 1]);
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
