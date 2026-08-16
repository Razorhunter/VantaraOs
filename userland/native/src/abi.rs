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
pub const SYS_THREAD_CREATE: u64 = 29;
pub const SYS_THREAD_EXIT: u64 = 30;
pub const SYS_THREAD_JOIN: u64 = 31;
pub const SYS_THREAD_SPAWN: u64 = 32;
pub const SYS_PIPE: u64 = 33;
pub const SYS_EVENT_CREATE: u64 = 34;
pub const SYS_EVENT_WAIT: u64 = 35;
pub const SYS_EVENT_SIGNAL: u64 = 36;
pub const SYS_EVENT_CLOSE: u64 = 37;
pub const SYS_MSGQ_CREATE: u64 = 38;
pub const SYS_MSGQ_SEND: u64 = 39;
pub const SYS_MSGQ_RECV: u64 = 40;
pub const SYS_MSGQ_CLOSE: u64 = 41;
pub const SYS_SIGPROCMASK: u64 = 42;
pub const SYS_SIGPENDING: u64 = 43;
pub const SYS_SIGACTION: u64 = 44;
pub const SYS_SIGRETURN: u64 = 45;
pub const SYS_GETPGRP: u64 = 46;
pub const SYS_SETPGID: u64 = 47;
pub const SYS_GETSID: u64 = 48;
pub const SYS_SETSID: u64 = 49;
pub const SYS_CREATE: u64 = 50;
pub const SYS_UNLINK: u64 = 51;
pub const SYS_RENAME: u64 = 52;
pub const SYS_MKDIR: u64 = 53;
pub const SYS_RMDIR: u64 = 54;
pub const SYS_UDP_BIND: u64 = 55;
pub const SYS_UDP_SEND_TO: u64 = 56;
pub const SYS_UDP_RECV_FROM: u64 = 57;
pub const SYS_UDP_CLOSE: u64 = 58;
pub const SYS_TCP_LISTEN: u64 = 59;
pub const SYS_TCP_ACCEPT: u64 = 60;
pub const SYS_TCP_CONNECT: u64 = 61;
pub const SYS_TCP_SEND: u64 = 62;
pub const SYS_TCP_RECV: u64 = 63;
pub const SYS_TCP_CLOSE: u64 = 64;
pub const SYS_SURFACE_CREATE: u64 = 65;
pub const SYS_SURFACE_CONFIGURE: u64 = 66;
pub const SYS_SURFACE_SET_COLOR: u64 = 67;
pub const SYS_SURFACE_FOCUS: u64 = 68;
pub const SYS_SURFACE_DESTROY: u64 = 69;
pub const SYS_SURFACE_DAMAGE: u64 = 70;
pub const SYS_FRAME_FENCE_STATUS: u64 = 71;

pub const ABI_VERSION_MAJOR: u64 = 1;
pub const ABI_VERSION_MINOR: u64 = 16;
pub const ABI_VERSION: u64 = (ABI_VERSION_MAJOR << 32) | ABI_VERSION_MINOR;

pub const ERR_UNKNOWN_SYSCALL: i64 = -1;
pub const ERR_INVALID_ARGUMENT: i64 = -2;
pub const ERR_NOT_IMPLEMENTED: i64 = -3;
pub const ERR_NO_SUCH_PROCESS: i64 = -4;
pub const ERR_NOT_CHILD: i64 = -5;
pub const ERR_WOULD_BLOCK: i64 = -6;
pub const ERR_PERMISSION_DENIED: i64 = -7;

pub const SIG_BLOCK: u64 = 0;
pub const SIG_UNBLOCK: u64 = 1;
pub const SIG_SETMASK: u64 = 2;
pub const SIGTERM: u64 = 15;
pub const SIGCONT: u64 = 18;
pub const SIGTSTP: u64 = 20;
pub const SIGINT: u64 = 2;
pub const SIGTERM_MASK: u64 = 1 << (SIGTERM - 1);
pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;

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
    assert!(SYS_THREAD_EXIT == 30);
    assert!(SYS_THREAD_JOIN == 31);
    assert!(SYS_THREAD_SPAWN == 32);
    assert!(SYS_PIPE == 33);
    assert!(SYS_EVENT_CLOSE == 37);
    assert!(SYS_MSGQ_CLOSE == 41);
    assert!(SYS_SIGPROCMASK == 42);
    assert!(SYS_SIGPENDING == 43);
    assert!(SYS_SIGACTION == 44);
    assert!(SYS_SIGRETURN == 45);
    assert!(SYS_GETPGRP == 46);
    assert!(SYS_SETPGID == 47);
    assert!(SYS_GETSID == 48);
    assert!(SYS_SETSID == 49);
    assert!(SYS_CREATE == 50);
    assert!(SYS_UNLINK == 51);
    assert!(SYS_RENAME == 52);
    assert!(SYS_MKDIR == 53);
    assert!(SYS_RMDIR == 54);
    assert!(SYS_UDP_BIND == 55);
    assert!(SYS_UDP_SEND_TO == 56);
    assert!(SYS_UDP_RECV_FROM == 57);
    assert!(SYS_UDP_CLOSE == 58);
    assert!(ERR_UNKNOWN_SYSCALL == -1);
    assert!(ERR_WOULD_BLOCK == -6);
    assert!(ERR_PERMISSION_DENIED == -7);
};

#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct FileStat {
    pub size: u64,
    pub readonly: u64,
    pub file_type: u64,
    pub inode: u64,
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct UdpDatagramMeta {
    pub source_ip: u32,
    pub source_port: u16,
    pub destination_port: u16,
    pub payload_len: u64,
}

pub fn write(text: &str) -> u64 {
    write_bytes(STDOUT, text.as_bytes())
}

pub fn debug(text: &str) -> u64 {
    write_bytes(STDERR, text.as_bytes())
}

pub fn write_line(text: &str) -> u64 {
    let written = write(text);
    write("\n");
    written
}

pub fn write_bytes(fd: u64, bytes: &[u8]) -> u64 {
    unsafe { syscall3(SYS_WRITE, fd, bytes.as_ptr() as u64, bytes.len() as u64) }
}

pub fn surface_create(x: u64, y: u64, width: u64, height: u64, color: u8) -> i64 {
    unsafe { syscall5(SYS_SURFACE_CREATE, x, y, width, height, u64::from(color)) as i64 }
}

pub fn surface_configure(id: u64, x: u64, y: u64, width: u64, height: u64) -> i64 {
    unsafe { syscall5(SYS_SURFACE_CONFIGURE, id, x, y, width, height) as i64 }
}

pub fn surface_set_color(id: u64, color: u8) -> i64 {
    unsafe { syscall2(SYS_SURFACE_SET_COLOR, id, u64::from(color)) as i64 }
}

pub fn surface_focus(id: u64) -> i64 {
    unsafe { syscall1(SYS_SURFACE_FOCUS, id) as i64 }
}

pub fn surface_destroy(id: u64) -> i64 {
    unsafe { syscall1(SYS_SURFACE_DESTROY, id) as i64 }
}

pub fn surface_damage(id: u64, x: u64, y: u64, width: u64, height: u64) -> i64 {
    unsafe { syscall5(SYS_SURFACE_DAMAGE, id, x, y, width, height) as i64 }
}

pub fn frame_fence_status(fence: u64) -> i64 {
    unsafe { syscall1(SYS_FRAME_FENCE_STATUS, fence) as i64 }
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

pub fn create(path: &[u8]) -> i64 {
    unsafe { syscall2(SYS_CREATE, path.as_ptr() as u64, path.len() as u64) as i64 }
}

pub fn unlink(path: &[u8]) -> i64 {
    unsafe { syscall2(SYS_UNLINK, path.as_ptr() as u64, path.len() as u64) as i64 }
}

pub fn rename(old_path: &[u8], new_path: &[u8]) -> i64 {
    unsafe {
        syscall4(
            SYS_RENAME,
            old_path.as_ptr() as u64,
            old_path.len() as u64,
            new_path.as_ptr() as u64,
            new_path.len() as u64,
        ) as i64
    }
}

pub fn mkdir(path: &[u8]) -> i64 {
    unsafe { syscall2(SYS_MKDIR, path.as_ptr() as u64, path.len() as u64) as i64 }
}

pub fn rmdir(path: &[u8]) -> i64 {
    unsafe { syscall2(SYS_RMDIR, path.as_ptr() as u64, path.len() as u64) as i64 }
}

pub fn pipe(fds: &mut [u64; 2]) -> i64 {
    unsafe {
        syscall2(
            SYS_PIPE,
            fds.as_mut_ptr() as u64,
            core::mem::size_of::<[u64; 2]>() as u64,
        ) as i64
    }
}

pub fn event_create() -> i64 {
    unsafe { syscall1(SYS_EVENT_CREATE, 0) as i64 }
}

pub fn event_wait(handle: u64) -> i64 {
    unsafe { syscall1(SYS_EVENT_WAIT, handle) as i64 }
}

pub fn event_signal(handle: u64) -> i64 {
    unsafe { syscall1(SYS_EVENT_SIGNAL, handle) as i64 }
}

pub fn event_close(handle: u64) -> i64 {
    unsafe { syscall1(SYS_EVENT_CLOSE, handle) as i64 }
}

pub fn msgq_create() -> i64 {
    unsafe { syscall1(SYS_MSGQ_CREATE, 0) as i64 }
}

pub fn msgq_send(handle: u64, message: &[u8]) -> i64 {
    unsafe {
        syscall3(
            SYS_MSGQ_SEND,
            handle,
            message.as_ptr() as u64,
            message.len() as u64,
        ) as i64
    }
}

pub fn msgq_receive(handle: u64, out: &mut [u8]) -> i64 {
    unsafe {
        syscall3(
            SYS_MSGQ_RECV,
            handle,
            out.as_mut_ptr() as u64,
            out.len() as u64,
        ) as i64
    }
}

pub fn msgq_close(handle: u64) -> i64 {
    unsafe { syscall1(SYS_MSGQ_CLOSE, handle) as i64 }
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

pub fn udp_bind(local_port: u16) -> i64 {
    unsafe { syscall1(SYS_UDP_BIND, local_port as u64) as i64 }
}

pub fn udp_send_to(
    handle: u64,
    destination_ip: [u8; 4],
    destination_port: u16,
    payload: &[u8],
) -> i64 {
    let ip = u32::from_be_bytes(destination_ip) as u64;
    unsafe {
        syscall5(
            SYS_UDP_SEND_TO,
            handle,
            ip,
            destination_port as u64,
            payload.as_ptr() as u64,
            payload.len() as u64,
        ) as i64
    }
}

pub fn udp_recv_from(handle: u64, out: &mut [u8], meta: &mut UdpDatagramMeta) -> i64 {
    unsafe {
        syscall5(
            SYS_UDP_RECV_FROM,
            handle,
            out.as_mut_ptr() as u64,
            out.len() as u64,
            (meta as *mut UdpDatagramMeta) as u64,
            core::mem::size_of::<UdpDatagramMeta>() as u64,
        ) as i64
    }
}

pub fn udp_close(handle: u64) -> i64 {
    unsafe { syscall1(SYS_UDP_CLOSE, handle) as i64 }
}

pub fn tcp_listen(port: u16) -> i64 {
    unsafe { syscall1(SYS_TCP_LISTEN, port as u64) as i64 }
}

pub fn tcp_accept(listener: u64) -> i64 {
    unsafe { syscall1(SYS_TCP_ACCEPT, listener) as i64 }
}

pub fn tcp_connect(ip: [u8; 4], port: u16, local_port: u16) -> i64 {
    unsafe {
        syscall3(
            SYS_TCP_CONNECT,
            u32::from_be_bytes(ip) as u64,
            port as u64,
            local_port as u64,
        ) as i64
    }
}

pub fn tcp_send(handle: u64, payload: &[u8]) -> i64 {
    unsafe {
        syscall3(
            SYS_TCP_SEND,
            handle,
            payload.as_ptr() as u64,
            payload.len() as u64,
        ) as i64
    }
}

pub fn tcp_receive(handle: u64, out: &mut [u8]) -> i64 {
    unsafe {
        syscall3(
            SYS_TCP_RECV,
            handle,
            out.as_mut_ptr() as u64,
            out.len() as u64,
        ) as i64
    }
}

pub fn tcp_close(handle: u64) -> i64 {
    unsafe { syscall1(SYS_TCP_CLOSE, handle) as i64 }
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

pub fn sigprocmask(how: u64, mask: u64) -> i64 {
    unsafe { syscall2(SYS_SIGPROCMASK, how, mask) as i64 }
}

pub fn sigpending() -> i64 {
    unsafe { syscall1(SYS_SIGPENDING, 0) as i64 }
}

pub fn sigaction(signal: u64, handler: extern "C" fn(u64) -> !) -> i64 {
    unsafe { syscall2(SYS_SIGACTION, signal, handler as usize as u64) as i64 }
}

pub fn sigaction_disposition(signal: u64, disposition: u64) -> i64 {
    unsafe { syscall2(SYS_SIGACTION, signal, disposition) as i64 }
}

pub fn sigreturn() -> ! {
    unsafe {
        syscall1(SYS_SIGRETURN, 0);
    }
    loop {
        core::hint::spin_loop();
    }
}

pub fn getpgrp() -> i64 {
    unsafe { syscall1(SYS_GETPGRP, 0) as i64 }
}

pub fn setpgid(pid: u64, process_group_id: u64) -> i64 {
    unsafe { syscall2(SYS_SETPGID, pid, process_group_id) as i64 }
}

pub fn getsid(pid: u64) -> i64 {
    unsafe { syscall1(SYS_GETSID, pid) as i64 }
}

pub fn setsid() -> i64 {
    unsafe { syscall1(SYS_SETSID, 0) as i64 }
}

pub fn yield_now() -> u64 {
    unsafe { syscall1(SYS_YIELD, 0) }
}

pub fn sleep_ms(duration_ms: u64) -> i64 {
    unsafe { syscall1(SYS_SLEEP_MS, duration_ms) as i64 }
}

pub fn thread_create(entry: extern "C" fn(u64) -> !, stack: &mut [u8], arg: u64) -> i64 {
    unsafe {
        syscall4(
            SYS_THREAD_CREATE,
            entry as usize as u64,
            stack.as_mut_ptr() as u64,
            stack.len() as u64,
            arg,
        ) as i64
    }
}

pub fn thread_exit() -> ! {
    unsafe {
        syscall1(SYS_THREAD_EXIT, 0);
    }

    loop {
        core::hint::spin_loop();
    }
}

pub fn thread_join(tid: u64) -> i64 {
    unsafe { syscall1(SYS_THREAD_JOIN, tid) as i64 }
}

pub fn thread_spawn(entry: extern "C" fn(u64) -> !, arg: u64) -> i64 {
    unsafe { syscall2(SYS_THREAD_SPAWN, entry as usize as u64, arg) as i64 }
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

unsafe fn syscall5(number: u64, arg0: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "int 0x80",
            inlateout("rax") number => ret,
            in("rdi") arg0,
            in("rsi") arg1,
            in("rdx") arg2,
            in("r10") arg3,
            in("r8") arg4,
            options(nostack, preserves_flags)
        );
    }
    ret
}
