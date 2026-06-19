use core::arch::asm;

pub const SYS_EXIT: u64 = 1;
pub const SYS_WRITE: u64 = 2;
pub const SYS_UPTIME: u64 = 3;
pub const SYS_GETUID: u64 = 4;
pub const SYS_GETGID: u64 = 5;
pub const SYS_WHOAMI: u64 = 6;
pub const SYS_LISTDIR: u64 = 7;
pub const SYS_READ_FILE: u64 = 8;
pub const SYS_STAT: u64 = 9;
pub const SYS_OPEN: u64 = 10;
pub const SYS_READ: u64 = 11;
pub const SYS_CLOSE: u64 = 12;
pub const SYS_EXEC: u64 = 13;
pub const SYS_WAIT: u64 = 14;
pub const SYS_WAITPID: u64 = SYS_WAIT;
pub const SYS_PROCS: u64 = 15;
pub const SYS_KILL: u64 = 16;
pub const SYS_YIELD: u64 = 17;
pub const SYS_EXEC_BG: u64 = 18;
pub const SYS_SLEEP_MS: u64 = 19;
pub const SYS_GETCWD: u64 = 20;
pub const SYS_CHDIR: u64 = 21;
pub const SYS_SETUSER: u64 = 22;
pub const SYS_REBOOT: u64 = 23;
pub const SYS_PCI_LIST: u64 = 24;
pub const SYS_NETDEV_LIST: u64 = 25;
pub const SYS_KLOG_READ: u64 = 26;
pub const SYS_DRIVER_STATUS: u64 = 27;
pub const SYS_ABI_INFO: u64 = 28;

pub const ABI_VERSION_MAJOR: u64 = 1;
pub const ABI_VERSION_MINOR: u64 = 0;
pub const ABI_VERSION: u64 = (ABI_VERSION_MAJOR << 32) | ABI_VERSION_MINOR;

pub const ERR_UNKNOWN_SYSCALL: i64 = -1;
pub const ERR_INVALID_ARGUMENT: i64 = -2;
pub const ERR_NOT_IMPLEMENTED: i64 = -3;
pub const ERR_NO_SUCH_PROCESS: i64 = -4;
pub const ERR_NOT_CHILD: i64 = -5;
pub const ERR_WOULD_BLOCK: i64 = -6;

pub const STDOUT: u64 = 1;
pub const STDERR: u64 = 2;
pub const STDIN: u64 = 0;

pub const USER_BASE: u64 = 0x1000000;
pub const ARG_ADDR: u64 = USER_BASE + 0x340;
pub const ARG_LEN_ADDR: u64 = USER_BASE + 0x378;
pub const ARG_MAX_LEN: usize = 32;

const _: () = {
    assert!(SYS_EXIT == 1);
    assert!(SYS_DRIVER_STATUS == 27);
    assert!(SYS_ABI_INFO == 28);
    assert!(ERR_UNKNOWN_SYSCALL == -1);
    assert!(ERR_WOULD_BLOCK == -6);
};

#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct FileStat {
    pub size: u64,
    pub readonly: u64,
    pub file_type: u64,
    pub inode: u64,
}

pub fn write(text: &str) -> u64 {
    write_bytes(STDOUT, text.as_bytes())
}

pub fn write_line(text: &str) -> u64 {
    let written = write(text);
    write("\n");
    written
}

pub fn write_bytes(fd: u64, bytes: &[u8]) -> u64 {
    unsafe { syscall3(SYS_WRITE, fd, bytes.as_ptr() as u64, bytes.len() as u64) }
}

pub fn uptime_ms() -> u64 {
    unsafe { syscall2(SYS_UPTIME, 0, 0) }
}

pub fn uptime_to_buffer(buffer: &mut [u8]) -> usize {
    unsafe { syscall2(SYS_UPTIME, buffer.as_mut_ptr() as u64, buffer.len() as u64) as usize }
}

pub fn whoami(out: &mut [u8]) -> usize {
    unsafe { syscall2(SYS_WHOAMI, out.as_mut_ptr() as u64, out.len() as u64) as usize }
}

pub fn getuid() -> u64 {
    unsafe { syscall1(SYS_GETUID, 0) }
}

pub fn getgid() -> u64 {
    unsafe { syscall1(SYS_GETGID, 0) }
}

pub fn setuser(name: &[u8]) -> i64 {
    unsafe { syscall2(SYS_SETUSER, name.as_ptr() as u64, name.len() as u64) as i64 }
}

pub fn reboot() -> ! {
    unsafe {
        syscall1(SYS_REBOOT, 0);
    }

    loop {
        core::hint::spin_loop();
    }
}

pub fn listdir(path: &str, out: &mut [u8]) -> usize {
    unsafe {
        syscall4(
            SYS_LISTDIR,
            path.as_ptr() as u64,
            path.len() as u64,
            out.as_mut_ptr() as u64,
            out.len() as u64,
        ) as usize
    }
}

pub fn read_file(path: &str, out: &mut [u8]) -> usize {
    unsafe {
        syscall4(
            SYS_READ_FILE,
            path.as_ptr() as u64,
            path.len() as u64,
            out.as_mut_ptr() as u64,
            out.len() as u64,
        ) as usize
    }
}

pub fn stat(path: &str, out: &mut [u8]) -> i64 {
    unsafe {
        syscall4(
            SYS_STAT,
            path.as_ptr() as u64,
            path.len() as u64,
            out.as_mut_ptr() as u64,
            out.len() as u64,
        ) as i64
    }
}

pub fn file_stat(path: &[u8], out: &mut FileStat) -> i64 {
    unsafe {
        syscall4(
            SYS_STAT,
            path.as_ptr() as u64,
            path.len() as u64,
            (out as *mut FileStat) as u64,
            core::mem::size_of::<FileStat>() as u64,
        ) as i64
    }
}

pub fn open(path: &str) -> i64 {
    unsafe { syscall2(SYS_OPEN, path.as_ptr() as u64, path.len() as u64) as i64 }
}

pub fn read(fd: u64, out: &mut [u8]) -> i64 {
    unsafe { syscall3(SYS_READ, fd, out.as_mut_ptr() as u64, out.len() as u64) as i64 }
}

pub fn close(fd: u64) -> i64 {
    unsafe { syscall1(SYS_CLOSE, fd) as i64 }
}

pub fn exec(path: &str, arg: Option<&str>) -> i64 {
    exec_bytes(path.as_bytes(), arg.map(str::as_bytes))
}

pub fn exec_background(path: &str, arg: Option<&str>) -> i64 {
    exec_background_bytes(path.as_bytes(), arg.map(str::as_bytes))
}

pub fn exec_bytes(path: &[u8], arg: Option<&[u8]>) -> i64 {
    exec_with_syscall(SYS_EXEC, path, arg)
}

pub fn exec_background_bytes(path: &[u8], arg: Option<&[u8]>) -> i64 {
    exec_with_syscall(SYS_EXEC_BG, path, arg)
}

pub fn waitpid(pid: u64) -> i64 {
    unsafe { syscall1(SYS_WAITPID, pid) as i64 }
}

pub fn procs(out: &mut [u8]) -> usize {
    unsafe { syscall2(SYS_PROCS, out.as_mut_ptr() as u64, out.len() as u64) as usize }
}

pub fn pci_list(out: &mut [u8]) -> usize {
    unsafe { syscall2(SYS_PCI_LIST, out.as_mut_ptr() as u64, out.len() as u64) as usize }
}

pub fn netdev_list(out: &mut [u8]) -> usize {
    unsafe { syscall2(SYS_NETDEV_LIST, out.as_mut_ptr() as u64, out.len() as u64) as usize }
}

pub fn kernel_log(out: &mut [u8]) -> usize {
    unsafe { syscall2(SYS_KLOG_READ, out.as_mut_ptr() as u64, out.len() as u64) as usize }
}

pub fn driver_status(out: &mut [u8]) -> usize {
    unsafe { syscall2(SYS_DRIVER_STATUS, out.as_mut_ptr() as u64, out.len() as u64) as usize }
}

pub fn abi_version() -> (u32, u32) {
    let version = unsafe { syscall1(SYS_ABI_INFO, 0) };
    ((version >> 32) as u32, version as u32)
}

pub fn kill(pid: u64, signal: u64) -> i64 {
    unsafe { syscall2(SYS_KILL, pid, signal) as i64 }
}

pub fn yield_now() -> u64 {
    unsafe { syscall1(SYS_YIELD, 0) }
}

pub fn sleep_ms(duration_ms: u64) -> i64 {
    unsafe { syscall1(SYS_SLEEP_MS, duration_ms) as i64 }
}

pub fn getcwd(out: &mut [u8]) -> i64 {
    unsafe { syscall2(SYS_GETCWD, out.as_mut_ptr() as u64, out.len() as u64) as i64 }
}

pub fn chdir(path: &[u8]) -> i64 {
    unsafe { syscall2(SYS_CHDIR, path.as_ptr() as u64, path.len() as u64) as i64 }
}

pub fn program_arg() -> &'static [u8] {
    let len = unsafe { core::ptr::read_volatile(ARG_LEN_ADDR as *const u64) as usize };
    let len = len.min(ARG_MAX_LEN);
    unsafe { core::slice::from_raw_parts(ARG_ADDR as *const u8, len) }
}

pub fn argv() -> Argv<'static> {
    Argv::new(program_arg())
}

pub fn first_arg() -> Option<&'static [u8]> {
    argv().next()
}

#[derive(Debug, Clone, Copy)]
pub struct Argv<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Argv<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    pub fn next_arg(&mut self) -> Option<&'a [u8]> {
        while self.cursor < self.bytes.len() && is_arg_separator(self.bytes[self.cursor]) {
            self.cursor += 1;
        }

        if self.cursor >= self.bytes.len() {
            return None;
        }

        let start = self.cursor;
        while self.cursor < self.bytes.len() && !is_arg_separator(self.bytes[self.cursor]) {
            self.cursor += 1;
        }

        Some(&self.bytes[start..self.cursor])
    }
}

impl<'a> Iterator for Argv<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        self.next_arg()
    }
}

pub fn exit(status: u64) -> ! {
    unsafe {
        syscall1(SYS_EXIT, status);
    }

    loop {
        core::hint::spin_loop();
    }
}

const fn is_arg_separator(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

fn exec_with_syscall(syscall: u64, path: &[u8], arg: Option<&[u8]>) -> i64 {
    let (arg_ptr, arg_len) = match arg {
        Some(arg) => (arg.as_ptr() as u64, arg.len() as u64),
        None => (0, 0),
    };
    unsafe {
        syscall4(
            syscall,
            path.as_ptr() as u64,
            path.len() as u64,
            arg_ptr,
            arg_len,
        ) as i64
    }
}

unsafe fn syscall1(number: u64, arg0: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") number => ret,
            in("rdi") arg0,
            options(nostack, preserves_flags)
        );
    }
    ret
}

unsafe fn syscall2(number: u64, arg0: u64, arg1: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") number => ret,
            in("rdi") arg0,
            in("rsi") arg1,
            options(nostack, preserves_flags)
        );
    }
    ret
}

unsafe fn syscall3(number: u64, arg0: u64, arg1: u64, arg2: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") number => ret,
            in("rdi") arg0,
            in("rsi") arg1,
            in("rdx") arg2,
            options(nostack, preserves_flags)
        );
    }
    ret
}

unsafe fn syscall4(number: u64, arg0: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") number => ret,
            in("rdi") arg0,
            in("rsi") arg1,
            in("rdx") arg2,
            in("r10") arg3,
            options(nostack, preserves_flags)
        );
    }
    ret
}
