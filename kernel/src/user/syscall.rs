use core::arch::global_asm;
use core::slice;
use core::str;
use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

pub const SYSCALL_INTERRUPT: u8 = 0x80;

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

pub const ABI_VERSION_MAJOR: u64 = 1;
pub const ABI_VERSION_MINOR: u64 = 1;
pub const ABI_VERSION: u64 = (ABI_VERSION_MAJOR << 32) | ABI_VERSION_MINOR;

pub const SYSCALL_RETURN_TO_KERNEL: u64 = u64::MAX;
const MAX_WRITE_LEN: u64 = 1024;
const MAX_PATH_LEN: u64 = 128;
const MAX_READ_LEN: u64 = 1024;
const MIN_USER_THREAD_STACK_SIZE: u64 = 1024;
const MAX_USER_THREAD_STACK_SIZE: u64 = 64 * 1024;
const MAX_OPEN_FILES: usize = 16;
const OPEN_FILE_BUFFER_SIZE: usize = 256;
const STDIN: u64 = 0;
const FIRST_USER_FD: u64 = 3;

static SYSCALL_TRAPS: AtomicU64 = AtomicU64::new(0);
static USER_YIELDS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
struct OpenFile {
    pid: crate::user::process::Pid,
    fd: u64,
    path: &'static str,
    data: OpenFileData,
    offset: usize,
}

#[derive(Debug, Clone, Copy)]
enum OpenFileData {
    Static(&'static [u8]),
    Buffered {
        bytes: [u8; OPEN_FILE_BUFFER_SIZE],
        len: usize,
    },
}

impl OpenFileData {
    fn len(&self) -> usize {
        match self {
            Self::Static(data) => data.len(),
            Self::Buffered { len, .. } => *len,
        }
    }

    fn copy_to_user(&self, offset: usize, out_ptr: u64, count: usize) {
        let source = match self {
            Self::Static(data) => &data[offset..offset + count],
            Self::Buffered { bytes, .. } => &bytes[offset..offset + count],
        };
        // SAFETY: syscall validation proved the complete destination range is
        // mapped user-writable; the source slice bounds are checked above.
        unsafe {
            core::ptr::copy_nonoverlapping(source.as_ptr(), out_ptr as *mut u8, count);
        }
    }
}

static OPEN_FILES: Mutex<[Option<OpenFile>; MAX_OPEN_FILES]> = Mutex::new([None; MAX_OPEN_FILES]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UserRange {
    ptr: u64,
    len: u64,
    end: u64,
    access: UserAccess,
}

impl UserRange {
    fn checked(ptr: u64, len: u64, access: UserAccess) -> Result<Self, SyscallError> {
        if ptr == 0 || len == 0 {
            return Err(SyscallError::InvalidArgument);
        }

        let end = ptr.checked_add(len).ok_or(SyscallError::InvalidArgument)?;
        let Some(layout) = crate::user::process::current_user_layout() else {
            return Err(SyscallError::InvalidArgument);
        };
        if !range_inside_layout(ptr, end, layout) {
            return Err(SyscallError::InvalidArgument);
        }

        let page_access = match access {
            UserAccess::Read => crate::user::address_space::UserPageAccess::Read,
            UserAccess::Write => crate::user::address_space::UserPageAccess::Write,
        };
        if !crate::user::address_space::active_user_range_accessible(ptr, len, page_access) {
            return Err(SyscallError::InvalidArgument);
        }

        Ok(Self {
            ptr,
            len,
            end,
            access,
        })
    }

    fn as_read_slice(self) -> &'static [u8] {
        debug_assert_eq!(self.access, UserAccess::Read);
        // SAFETY: UserRange::checked validates ownership, full extent, and
        // PRESENT|USER_ACCESSIBLE page-table permissions before construction.
        unsafe { slice::from_raw_parts(self.ptr as *const u8, self.len as usize) }
    }

    fn as_write_slice(self) -> &'static mut [u8] {
        debug_assert_eq!(self.access, UserAccess::Write);
        // SAFETY: UserRange::checked additionally requires WRITABLE on every
        // page. Syscall dispatch is single-owner for the active user process.
        unsafe { slice::from_raw_parts_mut(self.ptr as *mut u8, self.len as usize) }
    }
}

global_asm!(
    r#"
    .global syscall_interrupt_entry
syscall_interrupt_entry:
    push r15
    push r14
    push r13
    push r12
    push r11
    push r10
    push r9
    push r8
    push rbp
    push rdi
    push rsi
    push rdx
    push rcx
    push rbx
    push rax

    sub rsp, 192
    mov rax, [rsp + 192]
    mov [rsp + 0], rax
    mov rax, [rsp + 232]
    mov [rsp + 8], rax
    mov rax, [rsp + 224]
    mov [rsp + 16], rax
    mov rax, [rsp + 216]
    mov [rsp + 24], rax
    mov rax, [rsp + 264]
    mov [rsp + 32], rax
    mov rax, [rsp + 248]
    mov [rsp + 40], rax
    mov rax, [rsp + 256]
    mov [rsp + 48], rax
    mov rax, [rsp + 312]
    mov [rsp + 56], rax
    mov rax, [rsp + 336]
    mov [rsp + 64], rax
    mov rax, [rsp + 328]
    mov [rsp + 72], rax
    mov rax, [rsp + 200]
    mov [rsp + 80], rax
    mov rax, [rsp + 208]
    mov [rsp + 88], rax
    mov rax, [rsp + 216]
    mov [rsp + 96], rax
    mov rax, [rsp + 224]
    mov [rsp + 104], rax
    mov rax, [rsp + 232]
    mov [rsp + 112], rax
    mov rax, [rsp + 240]
    mov [rsp + 120], rax
    mov rax, [rsp + 248]
    mov [rsp + 128], rax
    mov rax, [rsp + 256]
    mov [rsp + 136], rax
    mov rax, [rsp + 264]
    mov [rsp + 144], rax
    mov rax, [rsp + 272]
    mov [rsp + 152], rax
    mov rax, [rsp + 280]
    mov [rsp + 160], rax
    mov rax, [rsp + 288]
    mov [rsp + 168], rax
    mov rax, [rsp + 296]
    mov [rsp + 176], rax
    mov rax, [rsp + 304]
    mov [rsp + 184], rax

    mov rdi, rsp
    call syscall_interrupt_dispatch

    cmp rax, -1
    je 2f

    mov [rsp + 192], rax
    add rsp, 192

    pop rax
    pop rbx
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rbp
    pop r8
    pop r9
    pop r10
    pop r11
    pop r12
    pop r13
    pop r14
    pop r15
    iretq

2:
    add rsp, 352
    call user_exit_landing
    "#
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallError {
    UnknownSyscall = -1,
    InvalidArgument = -2,
    NotImplemented = -3,
    NoSuchProcess = -4,
    NotChild = -5,
    WouldBlock = -6,
}

const _: () = {
    assert!(SYS_EXIT == 1);
    assert!(SYS_DRIVER_STATUS == 27);
    assert!(SYS_ABI_INFO == 28);
    assert!(SYS_THREAD_EXIT == 30);
    assert!(SyscallError::UnknownSyscall as i64 == -1);
    assert!(SyscallError::WouldBlock as i64 == -6);
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SyscallFrame {
    pub number: u64,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
    pub arg3: u64,
    pub arg4: u64,
    pub arg5: u64,
    pub user_rip: u64,
    pub user_rsp: u64,
    pub user_rflags: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct UserFileStat {
    pub size: u64,
    pub readonly: u64,
    pub file_type: u64,
    pub inode: u64,
}

pub fn init() {
    crate::serial_println!(
        "[SYSCALL] native ABI v{}.{} registered: int {:#x}, number=rax, args=rdi/rsi/rdx/r10/r8/r9, ret=rax",
        ABI_VERSION_MAJOR,
        ABI_VERSION_MINOR,
        SYSCALL_INTERRUPT,
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn syscall_interrupt_dispatch(frame: &SyscallFrame) -> u64 {
    let previous = SYSCALL_TRAPS.fetch_add(1, Ordering::Relaxed);
    if previous == 0 {
        crate::serial_println!("[SYSCALL] int 0x80 trap received from Ring-3");
    }

    if frame.number == SYS_EXIT {
        let pid = crate::user::process::mark_current_user_exited(frame.arg0);
        crate::serial_println!(
            "[SYSCALL] SYS_EXIT status={} pid={:?}; user task checked out",
            frame.arg0,
            pid
        );
        return SYSCALL_RETURN_TO_KERNEL;
    }

    if frame.number == SYS_THREAD_EXIT {
        let result = crate::user::process::mark_current_thread_exited();
        crate::serial_println!(
            "[SYSCALL] SYS_THREAD_EXIT pid_tid={:?}; thread checked out",
            result
        );
        return if result.is_some() {
            SYSCALL_RETURN_TO_KERNEL
        } else {
            SyscallError::InvalidArgument as i64 as u64
        };
    }

    if frame.number == SYS_WRITE {
        return match write_user_buffer(frame.arg0, frame.arg1, frame.arg2) {
            Ok(bytes_written) => bytes_written,
            Err(err) => err as i64 as u64,
        };
    }

    if frame.number == SYS_YIELD {
        return match user_yield(frame) {
            Ok(value) => value,
            Err(err) => err as i64 as u64,
        };
    }

    if frame.number == SYS_SLEEP_MS {
        return match sleep_current_user(frame) {
            Ok(value) => value,
            Err(err) => err as i64 as u64,
        };
    }

    if frame.number == SYS_REBOOT {
        crate::serial_println!("[SYSCALL] reboot requested from Ring-3");
        crate::power::reboot();
    }

    match dispatch(*frame) {
        Ok(value) => value,
        Err(err) => err as i64 as u64,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn user_exit_landing() -> ! {
    match unsafe { crate::user::address_space::restore_kernel_p4() } {
        Ok(frame) => {
            crate::serial_println!("[USER] restored kernel P4 {:#x}", frame);
        }
        Err(err) => {
            crate::serial_println!("[USER] kernel P4 restore skipped: {:?}", err);
        }
    }

    if let Some(blocked) = crate::user::process::checkout_current_user_if_blocked() {
        crate::serial_println!(
            "[USER] blocked pid={} {} waiting_for={:?} resume={:?}; returned to kernel scheduler",
            blocked.pid,
            blocked.name,
            blocked.waiting_for,
            blocked.resume_context
        );
        maintain_process_table();
        crate::interrupts::enable();
        crate::runtime::run_event_loop();
    }

    if crate::user::program::has_pending() {
        crate::serial_println!(
            "[USER] pending program queue has {} item(s); returned to kernel scheduler",
            crate::user::program::pending_count()
        );
        maintain_process_table();
        crate::interrupts::enable();
        crate::runtime::run_event_loop();
    }

    maintain_process_table();

    crate::interrupts::enable();
    if let Some(resume) = schedule_ready_user() {
        resume_scheduled_user(resume, resume.resume_context.rax);
    }

    let last_exit = crate::user::process::last_exit_summary();
    match last_exit {
        Some((pid, name, status)) => {
            crate::serial_println!("[USER] pid={} {} exited status={}", pid, name, status);
        }
        None => {
            crate::serial_println!("[USER] user task exited");
        }
    }
    crate::serial_println!("[USER] returned to kernel shell after SYS_EXIT");
    crate::interrupts::enable();
    update_user_shell_respawn(last_exit);
    maintain_process_table();
    if !crate::user::program::has_pending() {
        crate::shell::init();
    }
    crate::runtime::run_event_loop();
}

fn update_user_shell_respawn(last_exit: Option<(crate::user::process::Pid, &'static str, u64)>) {
    let Some((child_pid, _, _)) = last_exit else {
        return;
    };

    let Some(request) = crate::user::program::take_user_shell_wait_request_for_child(child_pid)
    else {
        return;
    };

    if let Some(resume) =
        crate::user::process::resume_waiting_parent(request.shell_pid, request.child_pid)
    {
        crate::serial_println!(
            "[USER] waitpid reaped pid={} {} status={} parent={}; resuming parent rip={:#x} rsp={:#x}",
            resume.child_pid,
            resume.child_name,
            resume.status,
            resume.parent_pid,
            resume.resume_context.rip,
            resume.resume_context.rsp
        );
        maintain_process_table();
        crate::interrupts::enable();
        if let Some(frame) = resume.p4_frame {
            crate::serial_println!(
                "[USER] switching to resumed parent pid={} P4 {:#x}",
                resume.parent_pid,
                frame
            );
            unsafe {
                crate::user::ring3::switch_to_user_p4_and_resume(
                    frame,
                    resume.resume_context,
                    resume.status,
                );
            }
        }
        unsafe {
            crate::user::ring3::resume_user(resume.resume_context, resume.status);
        }
    } else {
        match crate::user::process::reap_child(request.shell_pid, request.child_pid) {
            Some((pid, child_name, status)) => {
                crate::serial_println!(
                    "[USER] waitpid reaped pid={} {} status={} parent={}",
                    pid,
                    child_name,
                    status,
                    request.shell_pid
                );
            }
            None => {
                crate::serial_println!(
                    "[USER] waitpid reap skipped child={} parent={}",
                    request.child_pid,
                    request.shell_pid
                );
            }
        }
        if crate::user::process::mark_reaped(request.shell_pid) {
            crate::serial_println!(
                "[USER] wait bridge reaped blocked parent pid={}",
                request.shell_pid
            );
        }
    }
}

fn maintain_process_table() {
    reap_unwaited_zombies();
    reap_orphan_zombies();
    compact_process_history();
}

fn reap_orphan_zombies() {
    let reaped = crate::user::process::reap_orphan_zombies();
    if reaped > 0 {
        crate::serial_println!("[USER] init reaper collected {} orphan zombie(s)", reaped);
    }
}

fn reap_unwaited_zombies() {
    let reaped = crate::user::process::reap_unwaited_zombies();
    if reaped > 0 {
        crate::serial_println!("[USER] background reaper collected {} zombie(s)", reaped);
    }
}

fn compact_process_history() {
    let removed = crate::user::process::compact_reaped_history();
    if removed > 0 {
        crate::serial_println!("[USER] compacted {} old reaped process record(s)", removed);
    }
}

fn schedule_ready_user() -> Option<crate::user::process::ScheduledUserResume> {
    let resume = crate::user::process::schedule_next_ready_user()?;
    if let Some(frame) = resume.p4_frame {
        crate::serial_println!(
            "[USER] cooperative scheduler resuming pid={} {} rip={:#x} rsp={:#x} P4 {:#x}",
            resume.pid,
            resume.name,
            resume.resume_context.rip,
            resume.resume_context.rsp,
            frame
        );
    }

    Some(resume)
}

fn resume_scheduled_user(
    resume: crate::user::process::ScheduledUserResume,
    return_value: u64,
) -> ! {
    if let Some(frame) = resume.p4_frame {
        unsafe {
            crate::user::ring3::switch_to_user_p4_and_resume(
                frame,
                resume.resume_context,
                return_value,
            );
        }
    }

    unsafe {
        crate::user::ring3::resume_user(resume.resume_context, return_value);
    }
}

pub fn trap_count() -> u64 {
    SYSCALL_TRAPS.load(Ordering::Relaxed)
}

pub fn yield_count() -> u64 {
    USER_YIELDS.load(Ordering::Relaxed)
}

pub fn dispatch(frame: SyscallFrame) -> Result<u64, SyscallError> {
    match frame.number {
        SYS_UPTIME => uptime_to_user(frame.arg0, frame.arg1),
        SYS_GETUID => Ok(current_credentials().uid as u64),
        SYS_GETGID => Ok(current_credentials().gid as u64),
        SYS_WHOAMI => {
            let credentials = current_credentials();
            copy_to_user_buffer(frame.arg0, frame.arg1, credentials.username())
        }
        SYS_LISTDIR => list_dir_to_user(frame.arg0, frame.arg1, frame.arg2, frame.arg3),
        SYS_READ_FILE => read_file_to_user(frame.arg0, frame.arg1, frame.arg2, frame.arg3),
        SYS_STAT => stat_to_user(frame.arg0, frame.arg1, frame.arg2, frame.arg3),
        SYS_OPEN => open_file(frame.arg0, frame.arg1),
        SYS_READ => read_fd_to_user(frame.arg0, frame.arg1, frame.arg2),
        SYS_CLOSE => close_fd(frame.arg0),
        SYS_EXEC => exec_user_program(frame.arg0, frame.arg1, frame.arg2, frame.arg3),
        SYS_EXEC_BG => exec_background_user_program(frame.arg0, frame.arg1, frame.arg2, frame.arg3),
        SYS_WAITPID => wait_for_pending_child(&frame),
        SYS_PROCS => procs_to_user(frame.arg0, frame.arg1),
        SYS_KILL => kill_process_placeholder(frame.arg0, frame.arg1),
        SYS_GETCWD => getcwd_to_user(frame.arg0, frame.arg1),
        SYS_CHDIR => chdir_user(frame.arg0, frame.arg1),
        SYS_SETUSER => set_current_user(frame.arg0, frame.arg1),
        SYS_REBOOT => Err(SyscallError::NotImplemented),
        SYS_PCI_LIST => pci_list_to_user(frame.arg0, frame.arg1),
        SYS_NETDEV_LIST => netdev_list_to_user(frame.arg0, frame.arg1),
        SYS_KLOG_READ => kernel_log_to_user(frame.arg0, frame.arg1),
        SYS_DRIVER_STATUS => driver_status_to_user(frame.arg0, frame.arg1),
        SYS_ABI_INFO => Ok(ABI_VERSION),
        SYS_THREAD_CREATE => create_user_thread(frame.arg0, frame.arg1, frame.arg2, frame.arg3),
        SYS_THREAD_EXIT => Err(SyscallError::NotImplemented),
        SYS_SLEEP_MS => Err(SyscallError::NotImplemented),
        SYS_YIELD => Err(SyscallError::NotImplemented),
        SYS_EXIT => Err(SyscallError::NotImplemented),
        SYS_WRITE => write_user_buffer(frame.arg0, frame.arg1, frame.arg2),
        _ => Err(SyscallError::UnknownSyscall),
    }
}

fn create_user_thread(
    entry: u64,
    stack_ptr: u64,
    stack_len: u64,
    arg: u64,
) -> Result<u64, SyscallError> {
    if stack_len < MIN_USER_THREAD_STACK_SIZE || stack_len > MAX_USER_THREAD_STACK_SIZE {
        return Err(SyscallError::InvalidArgument);
    }

    let layout =
        crate::user::process::current_user_layout().ok_or(SyscallError::InvalidArgument)?;
    if entry < layout.program_start || entry >= layout.program_end {
        return Err(SyscallError::InvalidArgument);
    }
    UserRange::checked(entry, 1, UserAccess::Read)?;
    UserRange::checked(stack_ptr, stack_len, UserAccess::Write)?;

    let stack_top = stack_ptr
        .checked_add(stack_len)
        .ok_or(SyscallError::InvalidArgument)?;
    let aligned_top = stack_top & !0xf;
    let initial_rsp = aligned_top
        .checked_sub(8)
        .filter(|rsp| *rsp >= stack_ptr)
        .ok_or(SyscallError::InvalidArgument)?;

    crate::user::process::create_current_user_thread(entry, stack_ptr, stack_top, initial_rsp, arg)
        .map(u64::from)
        .ok_or(SyscallError::InvalidArgument)
}

fn user_yield(frame: &SyscallFrame) -> Result<u64, SyscallError> {
    let pid = crate::user::process::current_user_pid().unwrap_or(0);
    let tid = crate::user::process::current_user_tid().unwrap_or(0);
    let name = crate::user::process::current_user_name().unwrap_or("unknown");
    let has_work = crate::user::program::has_pending()
        || crate::user::process::has_ready_thread_except_current();
    if !has_work {
        return Ok(0);
    }

    let count = USER_YIELDS.fetch_add(1, Ordering::Relaxed) + 1;
    let recorded = crate::user::process::record_current_thread_yield();
    let preempt_requested = crate::user::process::consume_timer_preempt_request_for_yield();
    if count <= 16 || count % 256 == 0 {
        crate::serial_println!(
            "[USER] yield pid={} tid={} name={} count={} preempt={}",
            pid,
            tid,
            name,
            count,
            preempt_requested as u8
        );
    }
    if !recorded {
        crate::serial_println!(
            "[USER] yield pid={} tid={} was not recorded in process table",
            pid,
            tid
        );
    }

    let resume_context = crate::user::process::UserResumeContext::from_syscall_frame(frame);
    match crate::user::process::checkout_current_user_if_yielded(resume_context) {
        Some(report) => {
            crate::serial_println!(
                "[USER] yielded pid={} {} rip={:#x} rsp={:#x}; returned to kernel scheduler",
                report.pid,
                report.name,
                report.resume_context.rip,
                report.resume_context.rsp
            );
            Ok(SYSCALL_RETURN_TO_KERNEL)
        }
        None => Err(SyscallError::InvalidArgument),
    }
}

fn sleep_current_user(frame: &SyscallFrame) -> Result<u64, SyscallError> {
    let duration_ms = frame.arg0;
    if duration_ms == 0 {
        return Ok(0);
    }

    let ticks_to_sleep = duration_ms
        .saturating_mul(crate::timer::TIMER_HZ as u64)
        .div_ceil(1_000)
        .max(1);
    let wake_tick = crate::timer::ticks().saturating_add(ticks_to_sleep);
    let resume_context = crate::user::process::UserResumeContext::from_syscall_frame(frame);
    if !crate::user::process::mark_current_user_sleeping(wake_tick, resume_context) {
        return Err(SyscallError::InvalidArgument);
    }

    let pid = crate::user::process::current_user_pid().unwrap_or(0);
    crate::serial_println!(
        "[USER] sleep pid={} duration_ms={} wake_tick={}",
        pid,
        duration_ms,
        wake_tick
    );
    Ok(SYSCALL_RETURN_TO_KERNEL)
}

fn uptime_to_user(out_ptr: u64, out_len: u64) -> Result<u64, SyscallError> {
    let uptime = crate::timer::uptime_ms();
    if out_ptr == 0 && out_len == 0 {
        return Ok(uptime);
    }

    let mut buffer = [0u8; 20];
    let len = format_u64_decimal(uptime, &mut buffer);
    copy_to_user_buffer(out_ptr, out_len, &buffer[..len])
}

fn write_user_buffer(fd: u64, ptr: u64, len: u64) -> Result<u64, SyscallError> {
    if fd != 1 && fd != 2 {
        return Err(SyscallError::InvalidArgument);
    }

    if len > MAX_WRITE_LEN {
        return Err(SyscallError::InvalidArgument);
    }

    let bytes = read_user_bytes(ptr, len)?;
    match str::from_utf8(bytes) {
        Ok(text) => {
            crate::serial_print!("{}", text);
            crate::print!("{}", text);
        }
        Err(_) => {
            crate::serial_print!("[USER bytes]");
            crate::print!("[USER bytes]");
            for &byte in bytes {
                crate::serial_print!(" {:02x}", byte);
                crate::print!(" {:02x}", byte);
            }
            crate::serial_println!();
            crate::println!();
        }
    }

    Ok(len)
}

fn list_dir_to_user(
    path_ptr: u64,
    path_len: u64,
    out_ptr: u64,
    out_len: u64,
) -> Result<u64, SyscallError> {
    let path = read_user_str(path_ptr, path_len)?;
    let cwd = current_cwd();
    let entries =
        crate::fs::list_from(cwd.as_str(), path).map_err(|_| SyscallError::InvalidArgument)?;

    let mut written = 0u64;
    for entry in entries {
        written += copy_piece_to_user(out_ptr, out_len, written, entry.as_bytes())?;
        written += copy_piece_to_user(out_ptr, out_len, written, b"\n")?;
    }

    Ok(written)
}

fn read_file_to_user(
    path_ptr: u64,
    path_len: u64,
    out_ptr: u64,
    out_len: u64,
) -> Result<u64, SyscallError> {
    let path = read_user_str(path_ptr, path_len)?;
    let cwd = current_cwd();
    let data =
        crate::fs::read_from(cwd.as_str(), path).map_err(|_| SyscallError::InvalidArgument)?;
    copy_to_user_buffer(out_ptr, out_len, data)
}

fn stat_to_user(
    path_ptr: u64,
    path_len: u64,
    out_ptr: u64,
    out_len: u64,
) -> Result<u64, SyscallError> {
    let path = read_user_str(path_ptr, path_len)?;
    let cwd = current_cwd();
    let stat =
        crate::fs::stat_from(cwd.as_str(), path).map_err(|_| SyscallError::InvalidArgument)?;
    let user_stat = UserFileStat {
        size: stat.size as u64,
        readonly: stat.readonly as u64,
        file_type: match stat.file_type {
            crate::fs::FileType::File => 1,
            crate::fs::FileType::Directory => 2,
        },
        inode: stat.inode,
    };
    let bytes = unsafe {
        slice::from_raw_parts(
            (&user_stat as *const UserFileStat).cast::<u8>(),
            core::mem::size_of::<UserFileStat>(),
        )
    };
    copy_to_user_buffer(out_ptr, out_len, bytes)
}

fn getcwd_to_user(out_ptr: u64, out_len: u64) -> Result<u64, SyscallError> {
    let cwd = current_cwd();
    copy_to_user_buffer(out_ptr, out_len, cwd.as_str().as_bytes())
}

fn chdir_user(path_ptr: u64, path_len: u64) -> Result<u64, SyscallError> {
    let path = read_user_str(path_ptr, path_len)?;
    crate::user::process::change_current_working_directory(path)
        .map(|_| 0)
        .map_err(|_| SyscallError::InvalidArgument)
}

fn set_current_user(name_ptr: u64, name_len: u64) -> Result<u64, SyscallError> {
    let name = read_user_str(name_ptr, name_len)?;
    if trim_ascii_spaces(name.as_bytes()).is_empty() {
        return Err(SyscallError::InvalidArgument);
    }

    if crate::user::process::set_current_user_credentials(name) {
        Ok(0)
    } else {
        Err(SyscallError::InvalidArgument)
    }
}

fn current_credentials() -> crate::user::process::ProcessCredentials {
    crate::user::process::current_user_credentials()
        .unwrap_or_else(crate::user::process::ProcessCredentials::root)
}

fn trim_ascii_spaces(bytes: &[u8]) -> &[u8] {
    let mut start = 0usize;
    let mut end = bytes.len();

    while start < end && bytes[start] == b' ' {
        start += 1;
    }
    while end > start && bytes[end - 1] == b' ' {
        end -= 1;
    }

    &bytes[start..end]
}

fn open_file(path_ptr: u64, path_len: u64) -> Result<u64, SyscallError> {
    let path = read_user_str(path_ptr, path_len)?;
    let open_path = normalize_open_path(path)?;
    let data = open_data_for_path(open_path)?;
    let pid = crate::user::process::current_user_pid().ok_or(SyscallError::InvalidArgument)?;

    let mut files = OPEN_FILES.lock();
    let fd = next_fd_for_pid(&files, pid).ok_or(SyscallError::InvalidArgument)?;
    let slot = files
        .iter_mut()
        .find(|entry| entry.is_none())
        .ok_or(SyscallError::InvalidArgument)?;
    *slot = Some(OpenFile {
        pid,
        fd,
        path: open_path,
        data,
        offset: 0,
    });

    crate::serial_println!("[FD] pid={} open {} -> fd={}", pid, open_path, fd);
    Ok(fd)
}

fn current_cwd() -> crate::fs::NormalizedPath {
    crate::user::process::current_working_directory()
        .unwrap_or_else(crate::fs::NormalizedPath::root)
}

fn read_fd_to_user(fd: u64, out_ptr: u64, out_len: u64) -> Result<u64, SyscallError> {
    if out_len > MAX_READ_LEN {
        return Err(SyscallError::InvalidArgument);
    }
    let out = user_write_range(out_ptr, out_len)?;

    if fd == STDIN {
        return read_stdin_to_user(out_ptr, out_len);
    }

    let pid = crate::user::process::current_user_pid().ok_or(SyscallError::InvalidArgument)?;

    let mut files = OPEN_FILES.lock();
    let file = files
        .iter_mut()
        .filter_map(Option::as_mut)
        .find(|file| file.pid == pid && file.fd == fd)
        .ok_or(SyscallError::InvalidArgument)?;

    let remaining = file.data.len().saturating_sub(file.offset);
    let count = remaining.min(out_len as usize);
    file.data
        .copy_to_user(file.offset, out.as_ptr() as u64, count);
    file.offset += count;
    Ok(count as u64)
}

fn normalize_open_path(path: &str) -> Result<&'static str, SyscallError> {
    let cwd = current_cwd();
    crate::fs::stat_from(cwd.as_str(), path)
        .map(|stat| stat.path)
        .map_err(|_| SyscallError::InvalidArgument)
}

fn open_data_for_path(path: &'static str) -> Result<OpenFileData, SyscallError> {
    if let Ok(data) = crate::fs::read(path) {
        return Ok(OpenFileData::Static(data));
    }

    let entries = crate::fs::list(path).map_err(|_| SyscallError::InvalidArgument)?;
    let mut bytes = [0u8; OPEN_FILE_BUFFER_SIZE];
    let mut len = 0usize;
    for entry in entries {
        len = append_to_buffer(&mut bytes, len, entry.as_bytes())?;
        len = append_to_buffer(&mut bytes, len, b"\n")?;
    }

    Ok(OpenFileData::Buffered { bytes, len })
}

fn append_to_buffer(
    buffer: &mut [u8; OPEN_FILE_BUFFER_SIZE],
    offset: usize,
    data: &[u8],
) -> Result<usize, SyscallError> {
    let end = offset
        .checked_add(data.len())
        .ok_or(SyscallError::InvalidArgument)?;
    if end > buffer.len() {
        return Err(SyscallError::InvalidArgument);
    }
    buffer[offset..end].copy_from_slice(data);
    Ok(end)
}

fn close_fd(fd: u64) -> Result<u64, SyscallError> {
    let pid = crate::user::process::current_user_pid().ok_or(SyscallError::InvalidArgument)?;
    close_fd_for_pid(pid, fd).map(|_| 0)
}

fn read_stdin_to_user(out_ptr: u64, out_len: u64) -> Result<u64, SyscallError> {
    if out_len == 0 || out_len > MAX_READ_LEN {
        return Err(SyscallError::InvalidArgument);
    }

    user_write_range(out_ptr, out_len)?;

    let mut count = 0u64;
    while count < out_len {
        let Some(byte) = crate::input::dequeue_keyboard_byte() else {
            return if count == 0 {
                Err(SyscallError::WouldBlock)
            } else {
                Ok(count)
            };
        };

        unsafe {
            core::ptr::write((out_ptr + count) as *mut u8, byte);
        }
        count += 1;

        if byte == b'\n' {
            break;
        }
    }

    Ok(count)
}

fn exec_user_program(
    path_ptr: u64,
    path_len: u64,
    arg_ptr: u64,
    arg_len: u64,
) -> Result<u64, SyscallError> {
    let path = read_user_str(path_ptr, path_len)?;
    let arg = if arg_len == 0 {
        None
    } else {
        Some(read_user_str(arg_ptr, arg_len)?)
    };
    let pid = match crate::user::program::request_path_with_arg(path, arg) {
        Ok(pid) => pid,
        Err(err) => {
            crate::serial_println!(
                "[USER] exec failed path={} arg={:?} err={:?}",
                path,
                arg,
                err
            );
            return Err(SyscallError::InvalidArgument);
        }
    };
    crate::serial_println!(
        "[USER] exec requested {} arg={:?} child_pid={}",
        path,
        arg,
        pid
    );
    Ok(pid as u64)
}

fn exec_background_user_program(
    path_ptr: u64,
    path_len: u64,
    arg_ptr: u64,
    arg_len: u64,
) -> Result<u64, SyscallError> {
    let path = read_user_str(path_ptr, path_len)?;
    let arg = if arg_len == 0 {
        None
    } else {
        Some(read_user_str(arg_ptr, arg_len)?)
    };
    let pid = match crate::user::program::request_path_background(path, arg) {
        Ok(pid) => pid,
        Err(err) => {
            crate::serial_println!(
                "[USER] bg exec failed path={} arg={:?} err={:?}",
                path,
                arg,
                err
            );
            return Err(SyscallError::InvalidArgument);
        }
    };
    crate::serial_println!(
        "[USER] bg exec requested {} arg={:?} child_pid={}",
        path,
        arg,
        pid
    );
    Ok(pid as u64)
}

fn wait_for_pending_child(frame: &SyscallFrame) -> Result<u64, SyscallError> {
    let pid = u32::try_from(frame.arg0).map_err(|_| SyscallError::InvalidArgument)?;
    let parent_pid =
        crate::user::process::current_user_pid().ok_or(SyscallError::InvalidArgument)?;
    let parent_name = crate::user::process::current_user_name().unwrap_or("unknown");

    if let Some((child_pid, child_name, status)) = crate::user::process::reap_child(parent_pid, pid)
    {
        crate::serial_println!(
            "[USER] {} waitpid collected zombie pid={} {} status={} parent={}",
            parent_name,
            child_pid,
            child_name,
            status,
            parent_pid
        );
        return Ok(status);
    }

    if crate::user::program::pending_pid() != Some(pid) {
        let status = crate::user::process::wait_target_status(parent_pid, pid);
        match status {
            crate::user::process::WaitTargetStatus::Missing => {
                crate::serial_println!(
                    "[USER] {} waitpid rejected pid={} reason=no such process pending={:?}",
                    parent_name,
                    pid,
                    crate::user::program::pending_pid()
                );
                return Err(SyscallError::NoSuchProcess);
            }
            crate::user::process::WaitTargetStatus::NotChild { actual_parent } => {
                crate::serial_println!(
                    "[USER] {} waitpid rejected pid={} reason=not child parent={:?} current={}",
                    parent_name,
                    pid,
                    actual_parent,
                    parent_pid
                );
                return Err(SyscallError::NotChild);
            }
            crate::user::process::WaitTargetStatus::NotExited { state } => {
                crate::serial_println!(
                    "[USER] {} waitpid rejected pid={} reason=not exited state={:?} pending={:?}",
                    parent_name,
                    pid,
                    state,
                    crate::user::program::pending_pid()
                );
                return Err(SyscallError::WouldBlock);
            }
            crate::user::process::WaitTargetStatus::Reapable => {
                crate::serial_println!(
                    "[USER] {} waitpid rejected pid={} reason=reap race pending={:?}",
                    parent_name,
                    pid,
                    crate::user::program::pending_pid()
                );
                return Err(SyscallError::WouldBlock);
            }
        }
    }

    let resume_context = crate::user::process::UserResumeContext::from_syscall_frame(frame);
    if !crate::user::process::mark_waiting(parent_pid, pid, resume_context) {
        return Err(SyscallError::InvalidArgument);
    }
    crate::user::program::request_user_shell_wait(parent_pid, pid);
    crate::serial_println!(
        "[USER] {} pid={} waiting for queued child pid={}",
        parent_name,
        parent_pid,
        pid
    );
    Ok(SYSCALL_RETURN_TO_KERNEL)
}

fn procs_to_user(out_ptr: u64, out_len: u64) -> Result<u64, SyscallError> {
    if out_len == 0 || out_len > MAX_READ_LEN {
        return Err(SyscallError::InvalidArgument);
    }

    let out = user_write_range(out_ptr, out_len)?;
    let written = crate::user::process::write_processes_to_buffer(
        out,
        trap_count(),
        yield_count(),
        crate::user::program::pending_count(),
    );
    Ok(written.min(out.len()) as u64)
}

fn pci_list_to_user(out_ptr: u64, out_len: u64) -> Result<u64, SyscallError> {
    if out_len == 0 || out_len > MAX_READ_LEN {
        return Err(SyscallError::InvalidArgument);
    }

    let out = user_write_range(out_ptr, out_len)?;
    let written = crate::drivers::pci::write_devices_to_buffer(out);
    Ok(written.min(out.len()) as u64)
}

fn netdev_list_to_user(out_ptr: u64, out_len: u64) -> Result<u64, SyscallError> {
    if out_len == 0 || out_len > MAX_READ_LEN {
        return Err(SyscallError::InvalidArgument);
    }

    let out = user_write_range(out_ptr, out_len)?;
    let written = crate::drivers::network::write_devices_to_buffer(out);
    Ok(written.min(out.len()) as u64)
}

fn kernel_log_to_user(out_ptr: u64, out_len: u64) -> Result<u64, SyscallError> {
    if out_len == 0 || out_len > MAX_READ_LEN {
        return Err(SyscallError::InvalidArgument);
    }

    let out = user_write_range(out_ptr, out_len)?;
    Ok(crate::serial::copy_recent_log(out) as u64)
}

fn driver_status_to_user(out_ptr: u64, out_len: u64) -> Result<u64, SyscallError> {
    if out_len == 0 || out_len > MAX_READ_LEN {
        return Err(SyscallError::InvalidArgument);
    }

    let out = user_write_range(out_ptr, out_len)?;
    Ok(crate::drivers::status::write_to_buffer(out) as u64)
}

fn kill_process_placeholder(pid: u64, signal: u64) -> Result<u64, SyscallError> {
    let pid = u32::try_from(pid).map_err(|_| SyscallError::InvalidArgument)?;
    if !crate::user::process::contains_pid(pid) {
        crate::serial_println!(
            "[USER] kill placeholder rejected pid={} signal={} reason=no such process",
            pid,
            signal
        );
        return Err(SyscallError::NoSuchProcess);
    }

    crate::serial_println!(
        "[USER] kill placeholder accepted pid={} signal={} delivery=pending",
        pid,
        signal
    );
    Ok(0)
}

fn close_fd_for_pid(pid: crate::user::process::Pid, fd: u64) -> Result<(), SyscallError> {
    let mut files = OPEN_FILES.lock();
    let entry = files
        .iter_mut()
        .find(|entry| matches!(entry, Some(file) if file.pid == pid && file.fd == fd))
        .ok_or(SyscallError::InvalidArgument)?;
    if let Some(file) = entry.take() {
        crate::serial_println!("[FD] pid={} close fd={} path={}", pid, fd, file.path);
    }
    Ok(())
}

pub fn close_process_files(pid: crate::user::process::Pid) {
    let mut files = OPEN_FILES.lock();
    for entry in files.iter_mut() {
        if matches!(entry, Some(file) if file.pid == pid) {
            if let Some(file) = entry.take() {
                crate::serial_println!(
                    "[FD] pid={} auto-close fd={} path={}",
                    pid,
                    file.fd,
                    file.path
                );
            }
        }
    }
}

fn next_fd_for_pid(
    files: &[Option<OpenFile>; MAX_OPEN_FILES],
    pid: crate::user::process::Pid,
) -> Option<u64> {
    let mut fd = FIRST_USER_FD;
    loop {
        let taken = files
            .iter()
            .flatten()
            .any(|file| file.pid == pid && file.fd == fd);
        if !taken {
            return Some(fd);
        }
        fd += 1;
        if fd >= FIRST_USER_FD + MAX_OPEN_FILES as u64 {
            return None;
        }
    }
}

fn read_user_str(ptr: u64, len: u64) -> Result<&'static str, SyscallError> {
    if len > MAX_PATH_LEN {
        return Err(SyscallError::InvalidArgument);
    }

    let bytes = read_user_bytes(ptr, len)?;
    str::from_utf8(bytes).map_err(|_| SyscallError::InvalidArgument)
}

fn read_user_bytes(ptr: u64, len: u64) -> Result<&'static [u8], SyscallError> {
    Ok(user_read_range(ptr, len)?.as_read_slice())
}

fn copy_to_user_buffer(dst_ptr: u64, dst_len: u64, data: &[u8]) -> Result<u64, SyscallError> {
    if data.len() as u64 > dst_len {
        return Err(SyscallError::InvalidArgument);
    }

    let out = user_write_range(dst_ptr, data.len() as u64)?;
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), out.as_mut_ptr(), data.len());
    }
    Ok(data.len() as u64)
}

fn copy_piece_to_user(
    dst_ptr: u64,
    dst_len: u64,
    offset: u64,
    data: &[u8],
) -> Result<u64, SyscallError> {
    let next = offset
        .checked_add(data.len() as u64)
        .ok_or(SyscallError::InvalidArgument)?;
    if next > dst_len {
        return Err(SyscallError::InvalidArgument);
    }

    let out = user_write_range(dst_ptr + offset, data.len() as u64)?;
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), out.as_mut_ptr(), data.len());
    }
    Ok(data.len() as u64)
}

fn user_read_range(ptr: u64, len: u64) -> Result<UserRange, SyscallError> {
    UserRange::checked(ptr, len, UserAccess::Read)
}

fn user_write_range(ptr: u64, len: u64) -> Result<&'static mut [u8], SyscallError> {
    Ok(UserRange::checked(ptr, len, UserAccess::Write)?.as_write_slice())
}

fn range_inside_layout(ptr: u64, end: u64, layout: crate::user::ring3::UserMemoryLayout) -> bool {
    if end <= ptr {
        return false;
    }

    let in_program_area = ptr >= layout.program_start && end <= layout.program_end;
    let in_stack_area = ptr >= layout.stack_start && end <= layout.stack_top;
    in_program_area || in_stack_area
}

fn format_u64_decimal(mut value: u64, out: &mut [u8; 20]) -> usize {
    if value == 0 {
        out[0] = b'0';
        return 1;
    }

    let mut temp = [0u8; 20];
    let mut len = 0;
    while value > 0 {
        temp[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }

    for i in 0..len {
        out[i] = temp[len - 1 - i];
    }

    len
}

#[cfg(test)]
mod tests {
    use super::{
        ABI_VERSION, FIRST_USER_FD, OpenFile, OpenFileData, STDIN, SYS_ABI_INFO, SYS_GETGID,
        SYS_GETUID, SYS_LISTDIR, SYS_READ, SYS_UPTIME, SYS_WHOAMI, SYS_WRITE, SyscallError,
        SyscallFrame, dispatch, next_fd_for_pid,
    };

    fn frame(number: u64) -> SyscallFrame {
        SyscallFrame {
            number,
            arg0: 0,
            arg1: 0,
            arg2: 0,
            arg3: 0,
            arg4: 0,
            arg5: 0,
            user_rip: 0,
            user_rsp: 0,
            user_rflags: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
        }
    }

    #[test_case]
    fn unknown_syscall_returns_error() {
        let result = dispatch(frame(999));

        assert_eq!(result, Err(SyscallError::UnknownSyscall));
    }

    #[test_case]
    fn reports_native_abi_version() {
        assert_eq!(dispatch(frame(SYS_ABI_INFO)), Ok(ABI_VERSION));
    }

    #[test_case]
    fn allocates_first_user_file_descriptor_after_stdio() {
        let files = [None; super::MAX_OPEN_FILES];

        assert_eq!(next_fd_for_pid(&files, 2), Some(FIRST_USER_FD));
    }

    #[test_case]
    fn stdin_read_without_current_process_rejects_buffer() {
        let result = dispatch(SyscallFrame {
            arg0: STDIN,
            arg1: crate::user::ring3::FIRST_USER_ENTRY,
            arg2: 8,
            ..frame(SYS_READ)
        });

        assert_eq!(result, Err(SyscallError::InvalidArgument));
    }

    #[test_case]
    fn skips_file_descriptors_used_by_same_process() {
        let mut files = [None; super::MAX_OPEN_FILES];
        files[0] = Some(OpenFile {
            pid: 2,
            fd: FIRST_USER_FD,
            path: "/README",
            data: OpenFileData::Static(b""),
            offset: 0,
        });

        assert_eq!(next_fd_for_pid(&files, 2), Some(FIRST_USER_FD + 1));
        assert_eq!(next_fd_for_pid(&files, 3), Some(FIRST_USER_FD));
    }

    #[test_case]
    fn uptime_syscall_dispatches_successfully() {
        let result = dispatch(frame(SYS_UPTIME));

        assert!(result.is_ok());
    }

    #[test_case]
    fn decimal_formatter_handles_zero_and_large_values() {
        let mut buffer = [0u8; 20];

        let len = super::format_u64_decimal(0, &mut buffer);
        assert_eq!(&buffer[..len], b"0");

        let len = super::format_u64_decimal(123456789, &mut buffer);
        assert_eq!(&buffer[..len], b"123456789");
    }

    #[test_case]
    fn identity_syscalls_return_root_identity() {
        let uid = dispatch(frame(SYS_GETUID));
        let gid = dispatch(frame(SYS_GETGID));

        assert_eq!(uid, Ok(0));
        assert_eq!(gid, Ok(0));
    }

    #[test_case]
    fn write_rejects_invalid_fd() {
        let result = dispatch(SyscallFrame {
            arg0: 99,
            arg1: crate::user::ring3::FIRST_USER_MESSAGE_ADDR,
            arg2: crate::user::ring3::FIRST_USER_MESSAGE.len() as u64,
            ..frame(SYS_WRITE)
        });

        assert_eq!(result, Err(SyscallError::InvalidArgument));
    }

    #[test_case]
    fn user_buffer_syscalls_reject_out_of_range_pointers() {
        let whoami = dispatch(SyscallFrame {
            arg0: 0,
            arg1: 4,
            ..frame(SYS_WHOAMI)
        });
        let listdir = dispatch(SyscallFrame {
            arg0: 0,
            arg1: 1,
            arg2: crate::user::ring3::FIRST_USER_ENTRY,
            arg3: 32,
            ..frame(SYS_LISTDIR)
        });

        assert_eq!(whoami, Err(SyscallError::InvalidArgument));
        assert_eq!(listdir, Err(SyscallError::InvalidArgument));
    }

    #[test_case]
    fn user_range_rejects_null_empty_overflow_and_kernel_memory() {
        assert_eq!(
            super::UserRange::checked(0, 1, super::UserAccess::Read),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            super::UserRange::checked(
                crate::user::ring3::FIRST_USER_ENTRY,
                0,
                super::UserAccess::Read
            ),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            super::UserRange::checked(u64::MAX, 2, super::UserAccess::Read),
            Err(SyscallError::InvalidArgument)
        );
        assert_eq!(
            super::UserRange::checked(0xffff_8000_0000_0000, 8, super::UserAccess::Read),
            Err(SyscallError::InvalidArgument)
        );
    }

    #[test_case]
    fn layout_range_accepts_program_and_stack_memory() {
        let layout = crate::user::ring3::memory_layout_for_pid_with_program_size(2, 0x2000);
        assert!(super::range_inside_layout(
            layout.program_start,
            layout.program_start + 16,
            layout
        ));
        let stack_ptr = crate::user::ring3::FIRST_USER_STACK_TOP - 16;
        assert!(super::range_inside_layout(
            stack_ptr,
            crate::user::ring3::FIRST_USER_STACK_TOP,
            layout
        ));
    }

    #[test_case]
    fn layout_range_validates_cross_page_buffers_by_full_extent() {
        let layout = crate::user::ring3::memory_layout_for_pid_with_program_size(2, 0x2000);
        let cross_page_ptr = layout.program_start + 0xff0;
        assert!(super::range_inside_layout(
            cross_page_ptr,
            cross_page_ptr + 0x30,
            layout
        ));

        let past_program_end = layout.program_end - 0x10;
        assert!(!super::range_inside_layout(
            past_program_end,
            past_program_end + 0x30,
            layout
        ));

        let past_stack_top = layout.stack_top - 0x10;
        assert!(!super::range_inside_layout(
            past_stack_top,
            past_stack_top + 0x30,
            layout
        ));
    }
}
