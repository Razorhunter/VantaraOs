#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

const KERNEL_PTR: u64 = 0xffff_8000_0000_0000;
const FIRST_USER_STACK_TOP: u64 = 0x0140_0000;
const USER_STACK_SIZE: u64 = 0x4000;
const USER_STACK_SLOT_SIZE: u64 = 0x5000;
const USER_STACK_SLOT_COUNT: usize = 8;

#[allow(dead_code)]
mod abi {
    include!("../src/abi.rs");
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let arg = abi::program_arg();

    if eq(arg, b"invalidread") {
        invalid_read();
    } else if eq(arg, b"invalidwrite") || eq(arg, b"badptr") {
        invalid_write();
    } else if eq(arg, b"stackguard") {
        stack_guard_fault();
    } else if eq(arg, b"invalidopcode") {
        invalid_opcode();
    } else if eq(arg, b"generalprotection") {
        general_protection_fault();
    } else if eq(arg, b"dividezero") {
        divide_by_zero();
    } else {
        page_fault();
    }
}

fn invalid_read() -> ! {
    let ret = unsafe { raw_write_from_ptr(KERNEL_PTR, 8) };
    if ret < 0 {
        abi::write("fault: invalid read rejected\n");
        abi::exit(0);
    }

    abi::write("fault: invalid read was not rejected\n");
    abi::exit(1);
}

fn invalid_write() -> ! {
    let ret = unsafe { raw_whoami_to_ptr(KERNEL_PTR, 8) };
    if ret < 0 {
        abi::write("fault: invalid write rejected\n");
        abi::exit(0);
    }

    abi::write("fault: invalid write was not rejected\n");
    abi::exit(1);
}

fn stack_guard_fault() -> ! {
    let rsp: u64;
    unsafe {
        asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack, preserves_flags));
    }

    let mut stack_top = FIRST_USER_STACK_TOP;
    for _ in 0..USER_STACK_SLOT_COUNT {
        let stack_start = stack_top - USER_STACK_SIZE;
        if rsp >= stack_start && rsp < stack_top {
            unsafe {
                core::ptr::write_volatile((stack_start - 8) as *mut u64, 0xfeed_face);
            }
            abi::exit(1);
        }
        stack_top -= USER_STACK_SLOT_SIZE;
    }

    abi::write("fault: stack slot not found\n");
    abi::exit(1);
}

fn page_fault() -> ! {
    unsafe {
        core::ptr::read_volatile(core::ptr::null::<u64>());
    }
    abi::exit(1);
}

fn invalid_opcode() -> ! {
    unsafe {
        asm!("ud2", options(noreturn));
    }
}

fn general_protection_fault() -> ! {
    unsafe {
        asm!("hlt", options(noreturn));
    }
}

fn divide_by_zero() -> ! {
    unsafe {
        asm!(
            "mov rax, 1",
            "xor rdx, rdx",
            "xor rcx, rcx",
            "div rcx",
            options(noreturn)
        );
    }
}

fn eq(left: &[u8], right: &[u8]) -> bool {
    left == right
}

unsafe fn raw_whoami_to_ptr(ptr: u64, len: u64) -> i64 {
    let ret: u64;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") abi::SYS_WHOAMI => ret,
            in("rdi") ptr,
            in("rsi") len,
            options(nostack, preserves_flags)
        );
    }
    ret as i64
}

unsafe fn raw_write_from_ptr(ptr: u64, len: u64) -> i64 {
    let ret: u64;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") abi::SYS_WRITE => ret,
            in("rdi") abi::STDOUT,
            in("rsi") ptr,
            in("rdx") len,
            options(nostack, preserves_flags)
        );
    }
    ret as i64
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    abi::exit(1);
}
