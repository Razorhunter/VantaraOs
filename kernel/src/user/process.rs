use crate::sync::PreemptMutex as Mutex;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use lazy_static::lazy_static;

use crate::user::address_space::AddressSpace;
use crate::user::thread::{Thread, ThreadKind, ThreadState, ThreadStore, Tid};

pub type Pid = u32;
pub const DEFAULT_REAPED_HISTORY_LIMIT: usize = 16;
pub const USERNAME_MAX_LEN: usize = 16;
pub const SIGTERM: u64 = 15;
pub const SIGCONT: u64 = 18;
pub const SIGTSTP: u64 = 20;
pub const SIGINT: u64 = 2;
pub const SIGTERM_MASK: u64 = 1 << (SIGTERM - 1);
pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;
const USER_PREEMPT_QUANTUM_TICKS: u64 = 10;
static CURRENT_USER_PID_ATOMIC: AtomicU32 = AtomicU32::new(0);
static CURRENT_USER_TID_ATOMIC: AtomicU32 = AtomicU32::new(0);
static TIMER_PREEMPT_CHECKS: AtomicU64 = AtomicU64::new(0);
static TIMER_PREEMPT_REQUESTS: AtomicU64 = AtomicU64::new(0);
static TIMER_PREEMPT_YIELDS: AtomicU64 = AtomicU64::new(0);
static TIMER_PREEMPT_PENDING: AtomicBool = AtomicBool::new(false);
static LAST_PREEMPT_REQUEST_TICK: AtomicU64 = AtomicU64::new(0);
static TERMINAL_FOREGROUND_PID: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Created,
    Ready,
    Running,
    Blocked,
    Stopped,
    Exited,
    Zombie,
    Reaped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKind {
    KernelTask,
    UserProcess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHandle {
    pub id: u32,
}

#[derive(Debug)]
pub struct Process {
    pub pid: Pid,
    pub parent_pid: Option<Pid>,
    pub process_group_id: Pid,
    pub session_id: Pid,
    pub name: &'static str,
    pub program_path: &'static str,
    pub kind: ProcessKind,
    pub threads: ThreadStore,
    pub credentials: ProcessCredentials,
    pub arg: crate::user::program::UserProgramArg,
    pub cwd: crate::fs::NormalizedPath,
    pub job_mode: crate::user::program::JobMode,
    pub state: ProcessState,
    pub address_space: AddressSpace,
    pub file_handles: Vec<FileHandle>,
    pub exit_status: Option<u64>,
    pub waiting_for: Option<Pid>,
    pub sleeping_until_tick: Option<u64>,
    pub pending_signals: u64,
    pub blocked_signals: u64,
    pub sigterm_disposition: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessCredentials {
    pub uid: u32,
    pub gid: u32,
    username: [u8; USERNAME_MAX_LEN],
    username_len: usize,
}

impl ProcessCredentials {
    pub const fn root() -> Self {
        let mut username = [0; USERNAME_MAX_LEN];
        username[0] = b'r';
        username[1] = b'o';
        username[2] = b'o';
        username[3] = b't';
        Self {
            uid: 0,
            gid: 0,
            username,
            username_len: 4,
        }
    }

    pub fn from_username(name: &str) -> Self {
        let name = trim_ascii_spaces(name.as_bytes());
        if name == b"root" {
            return Self::root();
        }

        let mut username = [0; USERNAME_MAX_LEN];
        let len = name.len().min(USERNAME_MAX_LEN);
        username[..len].copy_from_slice(&name[..len]);
        Self {
            uid: 1000,
            gid: 1000,
            username,
            username_len: len,
        }
    }

    pub fn username(&self) -> &[u8] {
        &self.username[..self.username_len]
    }
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

pub const fn signal_bit(signal: u64) -> Option<u64> {
    if signal == 0 || signal > 64 {
        return None;
    }
    Some(1 << (signal - 1))
}

fn lowest_signal(mask: u64) -> Option<u64> {
    (mask != 0).then(|| u64::from(mask.trailing_zeros()) + 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessStats {
    pub process_count: usize,
    pub kernel_task_count: usize,
    pub user_process_count: usize,
    pub ready_user_count: usize,
    pub running_user_count: usize,
    pub blocked_user_count: usize,
    pub foreground_user_count: usize,
    pub background_user_count: usize,
    pub ready_count: usize,
    pub ready_queue_count: usize,
    pub running_count: usize,
    pub exited_count: usize,
    pub next_pid: Pid,
    pub user_context_switches: u64,
    pub ready_queue_skips: u64,
    pub timer_preempt_checks: u64,
    pub timer_preempt_requests: u64,
    pub timer_preempt_yields: u64,
    pub timer_preempt_pending: bool,
    pub thread_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecContext {
    pub pid: Pid,
    pub entry_point: u64,
    pub user_stack_top: u64,
    pub program_path: &'static str,
    pub arg: crate::user::program::UserProgramArg,
    pub layout: crate::user::ring3::UserMemoryLayout,
    pub p4_frame: Option<u64>,
    pub p4_verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserFaultReport {
    pub pid: Pid,
    pub name: &'static str,
    pub addr: u64,
    pub rip: u64,
    pub kind: UserFaultKind,
    pub status: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserExceptionReport {
    pub pid: Pid,
    pub name: &'static str,
    pub rip: u64,
    pub status: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalTerminationReport {
    pub pid: Pid,
    pub name: &'static str,
    pub parent_pid: Option<Pid>,
    pub job_mode: crate::user::program::JobMode,
    pub status: u64,
    pub parent_woken: bool,
    pub p4_frame: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalTerminationError {
    Missing,
    PermissionDenied,
    KernelTask,
    AlreadyExited,
    SelfTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalDelivery {
    Pending {
        pid: Pid,
        name: &'static str,
        signal: u64,
        pending_signals: u64,
    },
    Handled {
        pid: Pid,
        name: &'static str,
        signal: u64,
        handler: u64,
    },
    Ignored {
        pid: Pid,
        name: &'static str,
        signal: u64,
    },
    Terminated(SignalTerminationReport),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalMaskHow {
    Block,
    Unblock,
    SetMask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessGroupError {
    Missing,
    NotChild,
    DifferentSession,
    SessionLeader,
    GroupMissing,
    GroupLeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalMaskUpdate {
    pub old_mask: u64,
    pub new_mask: u64,
    pub delivered_signal: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalHandlerReport {
    pub pid: Pid,
    pub tid: Tid,
    pub signal: u64,
    pub handler: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSignalReport {
    pub pid: Pid,
    pub name: &'static str,
    pub signal: u64,
    pub status: u64,
    pub stopped: bool,
    pub parent_woken: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserFaultKind {
    Null,
    KernelSpace,
    StackGuard,
    OutsideUserRange,
    UserRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockedUserReport {
    pub pid: Pid,
    pub tid: Tid,
    pub name: &'static str,
    pub waiting_for: Option<Pid>,
    pub waiting_for_thread: Option<Tid>,
    pub resume_context: Option<UserResumeContext>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YieldedUserReport {
    pub pid: Pid,
    pub name: &'static str,
    pub resume_context: UserResumeContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledUserResume {
    pub pid: Pid,
    pub tid: Tid,
    pub name: &'static str,
    pub resume_context: UserResumeContext,
    pub p4_frame: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitResume {
    pub parent_pid: Pid,
    pub child_pid: Pid,
    pub child_name: &'static str,
    pub status: u64,
    pub resume_context: UserResumeContext,
    pub p4_frame: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitTargetStatus {
    Missing,
    NotChild { actual_parent: Option<Pid> },
    NotExited { state: ProcessState },
    Reapable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadJoinStatus {
    Missing,
    NotSibling,
    SelfJoin,
    Deadlock,
    AlreadyReaped,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct UserResumeContext {
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
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
    pub rax: u64,
}

const _: () = {
    assert!(core::mem::size_of::<UserResumeContext>() == 144);
    assert!(core::mem::offset_of!(UserResumeContext, rip) == 0);
    assert!(core::mem::offset_of!(UserResumeContext, rbx) == 24);
    assert!(core::mem::offset_of!(UserResumeContext, r10) == 88);
    assert!(core::mem::offset_of!(UserResumeContext, r15) == 128);
    assert!(core::mem::offset_of!(UserResumeContext, rax) == 136);
};

impl UserResumeContext {
    pub const EMPTY: Self = Self {
        rip: 0,
        rsp: 0,
        rflags: 0x202,
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
        rax: 0,
    };

    pub const fn from_syscall_frame(frame: &crate::user::syscall::SyscallFrame) -> Self {
        Self {
            rip: frame.user_rip,
            rsp: frame.user_rsp,
            rflags: frame.user_rflags,
            rbx: frame.rbx,
            rcx: frame.rcx,
            rdx: frame.rdx,
            rsi: frame.rsi,
            rdi: frame.rdi,
            rbp: frame.rbp,
            r8: frame.r8,
            r9: frame.r9,
            r10: frame.r10,
            r11: frame.r11,
            r12: frame.r12,
            r13: frame.r13,
            r14: frame.r14,
            r15: frame.r15,
            rax: 0,
        }
    }
}

impl Process {
    fn main_tid(&self) -> Tid {
        self.threads.main_tid()
    }

    fn main_thread(&self) -> &Thread {
        self.threads.main()
    }

    fn main_thread_mut(&mut self) -> &mut Thread {
        self.threads.main_mut()
    }

    fn set_state(&mut self, state: ProcessState) {
        self.state = state;
        let thread_state = thread_state_for_process(state);
        if matches!(
            state,
            ProcessState::Exited | ProcessState::Zombie | ProcessState::Reaped
        ) {
            for thread in self.threads.values_mut() {
                thread.state = thread_state;
                thread.resume_context = None;
                thread.last_started_tick = None;
            }
        } else {
            self.main_thread_mut().state = thread_state;
        }
    }

    fn refresh_state_from_threads(&mut self) {
        if matches!(
            self.state,
            ProcessState::Exited | ProcessState::Zombie | ProcessState::Reaped
        ) {
            return;
        }

        self.state = if self
            .threads
            .values()
            .any(|thread| thread.state == ThreadState::Running)
        {
            ProcessState::Running
        } else if self
            .threads
            .values()
            .any(|thread| thread.state == ThreadState::Ready)
        {
            ProcessState::Ready
        } else if self
            .threads
            .values()
            .any(|thread| thread.state == ThreadState::Blocked)
        {
            ProcessState::Blocked
        } else if self
            .threads
            .values()
            .any(|thread| thread.state == ThreadState::Stopped)
        {
            ProcessState::Stopped
        } else {
            ProcessState::Created
        };
    }
}

const fn thread_state_for_process(state: ProcessState) -> ThreadState {
    match state {
        ProcessState::Created => ThreadState::Created,
        ProcessState::Ready => ThreadState::Ready,
        ProcessState::Running => ThreadState::Running,
        ProcessState::Blocked => ThreadState::Blocked,
        ProcessState::Stopped => ThreadState::Stopped,
        ProcessState::Exited => ThreadState::Exited,
        ProcessState::Zombie => ThreadState::Zombie,
        ProcessState::Reaped => ThreadState::Reaped,
    }
}

pub struct ProcessStore {
    items: Vec<Process>,
}

impl ProcessStore {
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn insert(&mut self, pid: Pid, process: Process) {
        if let Some(slot) = self.items.iter_mut().find(|process| process.pid == pid) {
            *slot = process;
            return;
        }

        self.items.push(process);
    }

    pub fn get(&self, pid: &Pid) -> Option<&Process> {
        self.items.iter().find(|process| process.pid == *pid)
    }

    pub fn get_mut(&mut self, pid: &Pid) -> Option<&mut Process> {
        self.items.iter_mut().find(|process| process.pid == *pid)
    }

    pub fn remove(&mut self, pid: &Pid) -> Option<Process> {
        let index = self.items.iter().position(|process| process.pid == *pid)?;
        Some(self.items.remove(index))
    }

    pub fn contains_key(&self, pid: &Pid) -> bool {
        self.get(pid).is_some()
    }

    pub fn values(&self) -> core::slice::Iter<'_, Process> {
        self.items.iter()
    }

    pub fn get_by_tid(&self, tid: Tid) -> Option<&Process> {
        self.items
            .iter()
            .find(|process| process.threads.get(tid).is_some())
    }

    pub fn get_by_tid_mut(&mut self, tid: Tid) -> Option<&mut Process> {
        self.items
            .iter_mut()
            .find(|process| process.threads.get(tid).is_some())
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

pub struct ProcessTable {
    processes: ProcessStore,
    ready_queue: Vec<Tid>,
    next_pid: Pid,
    next_tid: Tid,
    user_context_switches: u64,
    ready_queue_skips: u64,
}

impl ProcessTable {
    pub const fn new() -> Self {
        Self {
            processes: ProcessStore::new(),
            ready_queue: Vec::new(),
            next_pid: 1,
            next_tid: 1,
            user_context_switches: 0,
            ready_queue_skips: 0,
        }
    }

    pub fn create_process(
        &mut self,
        parent_pid: Option<Pid>,
        name: &'static str,
        program_path: &'static str,
        arg: crate::user::program::UserProgramArg,
        address_space: AddressSpace,
    ) -> Pid {
        self.create_process_with_job_mode(
            parent_pid,
            name,
            program_path,
            arg,
            crate::user::program::JobMode::Foreground,
            address_space,
        )
    }

    pub fn create_process_with_job_mode(
        &mut self,
        parent_pid: Option<Pid>,
        name: &'static str,
        program_path: &'static str,
        arg: crate::user::program::UserProgramArg,
        job_mode: crate::user::program::JobMode,
        address_space: AddressSpace,
    ) -> Pid {
        let pid = self.reserve_pid();
        self.insert_process(
            pid,
            parent_pid,
            name,
            program_path,
            arg,
            job_mode,
            address_space,
        );
        pid
    }

    pub fn reserve_pid(&mut self) -> Pid {
        let pid = self.next_pid;
        self.next_pid += 1;
        pid
    }

    pub fn reserve_tid(&mut self) -> Tid {
        let tid = self.next_tid;
        self.next_tid = self.next_tid.saturating_add(1);
        tid
    }

    pub fn create_reserved_process(
        &mut self,
        pid: Pid,
        parent_pid: Option<Pid>,
        name: &'static str,
        program_path: &'static str,
        arg: crate::user::program::UserProgramArg,
        address_space: AddressSpace,
    ) -> Pid {
        self.create_reserved_process_with_job_mode(
            pid,
            parent_pid,
            name,
            program_path,
            arg,
            crate::user::program::JobMode::Foreground,
            address_space,
        )
    }

    pub fn create_reserved_process_with_job_mode(
        &mut self,
        pid: Pid,
        parent_pid: Option<Pid>,
        name: &'static str,
        program_path: &'static str,
        arg: crate::user::program::UserProgramArg,
        job_mode: crate::user::program::JobMode,
        address_space: AddressSpace,
    ) -> Pid {
        if pid >= self.next_pid {
            self.next_pid = pid + 1;
        }

        self.insert_process(
            pid,
            parent_pid,
            name,
            program_path,
            arg,
            job_mode,
            address_space,
        );
        pid
    }

    fn insert_process(
        &mut self,
        pid: Pid,
        parent_pid: Option<Pid>,
        name: &'static str,
        program_path: &'static str,
        arg: crate::user::program::UserProgramArg,
        job_mode: crate::user::program::JobMode,
        address_space: AddressSpace,
    ) {
        let kind = if parent_pid.is_none() && name == "kernel" && program_path == "kernel" {
            ProcessKind::KernelTask
        } else {
            ProcessKind::UserProcess
        };
        let cwd = parent_pid
            .and_then(|pid| self.processes.get(&pid).map(|process| process.cwd))
            .unwrap_or_else(crate::fs::NormalizedPath::root);
        let credentials = parent_pid
            .and_then(|pid| self.processes.get(&pid).map(|process| process.credentials))
            .unwrap_or_else(ProcessCredentials::root);
        let parent_job = parent_pid.and_then(|pid| {
            self.processes
                .get(&pid)
                .map(|process| (process.kind, process.process_group_id, process.session_id))
        });
        let (process_group_id, session_id) = match parent_job {
            Some((ProcessKind::UserProcess, pgid, sid)) => (pgid, sid),
            _ => (pid, pid),
        };
        let thread_kind = match kind {
            ProcessKind::KernelTask => ThreadKind::Kernel,
            ProcessKind::UserProcess => ThreadKind::User,
        };
        let main_tid = self.reserve_tid();

        self.processes.insert(
            pid,
            Process {
                pid,
                parent_pid,
                process_group_id,
                session_id,
                name,
                program_path,
                kind,
                threads: ThreadStore::with_main(
                    Thread::new(main_tid, pid, thread_kind).with_user_stack(
                        address_space.layout.stack_start,
                        address_space.layout.stack_top,
                    ),
                ),
                credentials,
                arg,
                cwd,
                job_mode,
                state: ProcessState::Created,
                address_space,
                file_handles: Vec::new(),
                exit_status: None,
                waiting_for: None,
                sleeping_until_tick: None,
                pending_signals: 0,
                blocked_signals: 0,
                sigterm_disposition: SIG_DFL,
            },
        );
    }

    pub fn credentials_for(&self, pid: Pid) -> Option<ProcessCredentials> {
        self.processes.get(&pid).map(|process| process.credentials)
    }

    pub fn process_group_id(&self, pid: Pid) -> Option<Pid> {
        self.processes
            .get(&pid)
            .map(|process| process.process_group_id)
    }

    pub fn session_id(&self, pid: Pid) -> Option<Pid> {
        self.processes.get(&pid).map(|process| process.session_id)
    }

    pub fn set_process_group(
        &mut self,
        caller_pid: Pid,
        target_pid: Pid,
        process_group_id: Pid,
    ) -> Result<Pid, ProcessGroupError> {
        let caller = self
            .processes
            .get(&caller_pid)
            .ok_or(ProcessGroupError::Missing)?;
        let caller_session = caller.session_id;
        let target = self
            .processes
            .get(&target_pid)
            .ok_or(ProcessGroupError::Missing)?;
        if target_pid != caller_pid && target.parent_pid != Some(caller_pid) {
            return Err(ProcessGroupError::NotChild);
        }
        if target.session_id != caller_session {
            return Err(ProcessGroupError::DifferentSession);
        }
        if target.session_id == target.pid {
            return Err(ProcessGroupError::SessionLeader);
        }

        let process_group_id = if process_group_id == 0 {
            target_pid
        } else {
            process_group_id
        };
        if process_group_id != target_pid
            && !self.processes.values().any(|process| {
                process.session_id == caller_session && process.process_group_id == process_group_id
            })
        {
            return Err(ProcessGroupError::GroupMissing);
        }

        let target = self
            .processes
            .get_mut(&target_pid)
            .expect("validated process-group target must remain present");
        target.process_group_id = process_group_id;
        Ok(process_group_id)
    }

    pub fn create_session(&mut self, pid: Pid) -> Result<Pid, ProcessGroupError> {
        let process = self.processes.get(&pid).ok_or(ProcessGroupError::Missing)?;
        if process.process_group_id == pid {
            return Err(ProcessGroupError::GroupLeader);
        }
        let process = self
            .processes
            .get_mut(&pid)
            .expect("validated session target must remain present");
        process.session_id = pid;
        process.process_group_id = pid;
        Ok(pid)
    }

    pub fn create_thread(&mut self, pid: Pid) -> Option<Tid> {
        let kind = match self.processes.get(&pid)?.kind {
            ProcessKind::KernelTask => ThreadKind::Kernel,
            ProcessKind::UserProcess => ThreadKind::User,
        };
        let tid = self.reserve_tid();
        let process = self.processes.get_mut(&pid)?;
        process
            .threads
            .insert(Thread::new(tid, pid, kind))
            .then_some(tid)
    }

    pub fn create_kernel_thread(&mut self) -> Option<Tid> {
        let pid = self
            .processes
            .values()
            .find(|process| process.kind == ProcessKind::KernelTask)?
            .pid;
        let tid = self.reserve_tid();
        let process = self.processes.get_mut(&pid)?;
        let mut thread = Thread::new(tid, pid, ThreadKind::Kernel);
        thread.state = ThreadState::Ready;
        if !process.threads.insert(thread) {
            return None;
        }
        process.refresh_state_from_threads();
        Some(tid)
    }

    pub fn set_kernel_thread_state(&mut self, tid: Tid, state: ThreadState) -> bool {
        let Some(process) = self.processes.get_by_tid_mut(tid) else {
            return false;
        };
        if process.kind != ProcessKind::KernelTask {
            return false;
        }
        let Some(thread) = process.threads.get_mut(tid) else {
            return false;
        };
        if thread.state == ThreadState::Running && state != ThreadState::Running {
            stop_thread_run(thread);
        }
        if state == ThreadState::Running {
            thread.context_switches = thread.context_switches.saturating_add(1);
            thread.last_started_tick = Some(crate::timer::ticks());
        }
        thread.state = state;
        process.refresh_state_from_threads();
        true
    }

    pub fn kernel_thread_state(&self, tid: Tid) -> Option<ThreadState> {
        let process = self.processes.get_by_tid(tid)?;
        (process.kind == ProcessKind::KernelTask)
            .then(|| process.threads.get(tid).map(|thread| thread.state))
            .flatten()
    }

    pub fn kernel_thread_runtime_ticks(&self, tid: Tid) -> Option<u64> {
        let process = self.processes.get_by_tid(tid)?;
        let thread = process.threads.get(tid)?;
        (process.kind == ProcessKind::KernelTask).then_some(thread.runtime_ticks)
    }

    pub fn create_user_thread(
        &mut self,
        pid: Pid,
        entry: u64,
        stack_start: u64,
        stack_top: u64,
        initial_rsp: u64,
        arg: u64,
    ) -> Option<Tid> {
        let process = self.processes.get(&pid)?;
        if process.kind != ProcessKind::UserProcess
            || !process
                .threads
                .stack_range_available(stack_start, stack_top)
        {
            return None;
        }

        let tid = self.reserve_tid();
        let process = self.processes.get_mut(&pid)?;
        let mut thread =
            Thread::new(tid, pid, ThreadKind::User).with_user_stack(stack_start, stack_top);
        thread.state = ThreadState::Ready;
        thread.resume_context = Some(UserResumeContext {
            rip: entry,
            rsp: initial_rsp,
            rflags: 0x202,
            rdi: arg,
            ..UserResumeContext::EMPTY
        });
        if !process.threads.insert(thread) {
            return None;
        }
        process.refresh_state_from_threads();
        self.enqueue_ready(tid);
        Some(tid)
    }

    pub fn create_managed_user_thread(&mut self, pid: Pid, entry: u64, arg: u64) -> Option<Tid> {
        let process = self.processes.get(&pid)?;
        if process.kind != ProcessKind::UserProcess {
            return None;
        }
        let p4_frame = process.address_space.p4_frame?;
        let (stack_start, stack_top) = crate::user::address_space::reserve_private_thread_stack(
            p4_frame,
            process.address_space.layout.stack_top,
        )?;
        let initial_rsp = (stack_top & !0xf).checked_sub(8)?;

        let tid = self.reserve_tid();
        let process = self.processes.get_mut(&pid)?;
        let mut thread = Thread::new(tid, pid, ThreadKind::User)
            .with_user_stack(stack_start, stack_top)
            .with_kernel_managed_stack();
        thread.state = ThreadState::Ready;
        thread.resume_context = Some(UserResumeContext {
            rip: entry,
            rsp: initial_rsp,
            rflags: 0x202,
            rdi: arg,
            ..UserResumeContext::EMPTY
        });
        if !process.threads.insert(thread) {
            let _ = crate::user::address_space::release_private_thread_stack(p4_frame, stack_top);
            return None;
        }
        process.refresh_state_from_threads();
        self.enqueue_ready(tid);
        Some(tid)
    }

    pub fn mark_thread_exited(&mut self, tid: Tid) -> Option<Pid> {
        let process = self.processes.get_by_tid_mut(tid)?;
        if tid == process.main_tid() {
            return None;
        }
        let thread = process.threads.get_mut(tid)?;
        stop_thread_run(thread);
        thread.state = ThreadState::Exited;
        thread.resume_context = None;
        thread.waiting_for = None;

        let mut woken = Vec::new();
        for waiter in process.threads.values_mut() {
            if waiter.state == ThreadState::Blocked && waiter.waiting_for == Some(tid) {
                waiter.waiting_for = None;
                waiter.state = ThreadState::Ready;
                if let Some(context) = waiter.resume_context.as_mut() {
                    context.rax = 0;
                }
                woken.push(waiter.tid);
            }
        }
        let reaped_stack = if !woken.is_empty() {
            process
                .threads
                .get_mut(tid)
                .expect("exiting thread must remain in its process")
                .state = ThreadState::Reaped;
            take_managed_stack_release(process, tid)
        } else {
            None
        };
        process.refresh_state_from_threads();
        let pid = process.pid;
        self.ready_queue.retain(|queued_tid| *queued_tid != tid);
        for waiter_tid in woken {
            self.enqueue_ready(waiter_tid);
        }
        if let Some((p4_frame, stack_top)) = reaped_stack {
            let _ = crate::user::address_space::release_private_thread_stack(p4_frame, stack_top);
        }
        Some(pid)
    }

    pub fn join_thread(
        &mut self,
        caller_tid: Tid,
        target_tid: Tid,
        resume_context: UserResumeContext,
    ) -> ThreadJoinStatus {
        if caller_tid == target_tid {
            return ThreadJoinStatus::SelfJoin;
        }

        let Some(caller_pid) = self
            .processes
            .get_by_tid(caller_tid)
            .map(|process| process.pid)
        else {
            return ThreadJoinStatus::Missing;
        };
        let Some(target_process) = self.processes.get_by_tid(target_tid) else {
            return ThreadJoinStatus::Missing;
        };
        if target_process.pid != caller_pid {
            return ThreadJoinStatus::NotSibling;
        }
        let target_state = target_process
            .threads
            .get(target_tid)
            .map(|thread| thread.state)
            .expect("TID lookup must resolve inside its owning process");
        if target_state == ThreadState::Reaped {
            return ThreadJoinStatus::AlreadyReaped;
        }
        if target_process
            .threads
            .get(target_tid)
            .is_some_and(|thread| thread.waiting_for == Some(caller_tid))
        {
            return ThreadJoinStatus::Deadlock;
        }

        let process = self
            .processes
            .get_mut(&caller_pid)
            .expect("caller process must remain present");
        if target_state == ThreadState::Exited {
            process
                .threads
                .get_mut(target_tid)
                .expect("target thread must remain present")
                .state = ThreadState::Reaped;
            if let Some((p4_frame, stack_top)) = take_managed_stack_release(process, target_tid) {
                let _ =
                    crate::user::address_space::release_private_thread_stack(p4_frame, stack_top);
            }
            return ThreadJoinStatus::Completed;
        }

        let caller = process
            .threads
            .get_mut(caller_tid)
            .expect("caller thread must remain present");
        if caller.state != ThreadState::Running {
            return ThreadJoinStatus::Missing;
        }
        stop_thread_run(caller);
        caller.state = ThreadState::Blocked;
        caller.waiting_for = Some(target_tid);
        caller.resume_context = Some(resume_context);
        process.refresh_state_from_threads();
        ThreadJoinStatus::Blocked
    }

    pub fn block_thread(&mut self, tid: Tid, resume_context: UserResumeContext) -> bool {
        let Some(process) = self.processes.get_by_tid_mut(tid) else {
            return false;
        };
        let Some(thread) = process.threads.get_mut(tid) else {
            return false;
        };
        if thread.state != ThreadState::Running {
            return false;
        }

        stop_thread_run(thread);
        thread.state = ThreadState::Blocked;
        thread.waiting_for = None;
        thread.resume_context = Some(resume_context);
        process.refresh_state_from_threads();
        true
    }

    pub fn wake_thread(&mut self, tid: Tid, return_value: u64) -> bool {
        let Some(process) = self.processes.get_by_tid_mut(tid) else {
            return false;
        };
        let Some(thread) = process.threads.get_mut(tid) else {
            return false;
        };
        if thread.state != ThreadState::Blocked {
            return false;
        }

        let Some(context) = thread.resume_context.as_mut() else {
            return false;
        };
        context.rax = return_value;
        thread.waiting_for = None;
        thread.state = ThreadState::Ready;
        process.refresh_state_from_threads();
        self.enqueue_ready(tid);
        true
    }

    pub fn thread(&self, tid: Tid) -> Option<&Thread> {
        self.processes.get_by_tid(tid)?.threads.get(tid)
    }

    pub fn set_credentials(&mut self, pid: Pid, credentials: ProcessCredentials) -> bool {
        let Some(process) = self.processes.get_mut(&pid) else {
            return false;
        };
        process.credentials = credentials;
        true
    }

    pub fn mark_ready(&mut self, pid: Pid) {
        if let Some(process) = self.processes.get_mut(&pid) {
            process.set_state(ProcessState::Ready);
            let tid = process.main_tid();
            self.enqueue_ready(tid);
        }
    }

    pub fn mark_running(&mut self, pid: Pid) {
        self.remove_from_ready_queue(pid);
        self.start_process_run(pid);
    }

    pub fn mark_running_fresh(&mut self, pid: Pid) {
        self.start_process_run(pid);
    }

    pub fn mark_exited(&mut self, pid: Pid, status: u64) {
        self.remove_from_ready_queue(pid);
        if let Some(process) = self.processes.get_mut(&pid) {
            stop_process_run(process);
            process.exit_status = Some(status);
            process.sleeping_until_tick = None;
            let state = if process.parent_pid.is_some() {
                ProcessState::Zombie
            } else {
                ProcessState::Exited
            };
            process.set_state(state);
        }
    }

    pub fn stop_user_process(&mut self, pid: Pid, signal: u64) -> Option<TerminalSignalReport> {
        self.remove_from_ready_queue(pid);
        let process = self.processes.get_mut(&pid)?;
        if process.kind != ProcessKind::UserProcess
            || matches!(
                process.state,
                ProcessState::Exited
                    | ProcessState::Zombie
                    | ProcessState::Reaped
                    | ProcessState::Stopped
            )
        {
            return None;
        }
        stop_process_run(process);
        process.sleeping_until_tick = None;
        process.set_state(ProcessState::Stopped);
        let parent_pid = process.parent_pid;
        let name = process.name;
        let status = 128 + signal;

        let mut parent_woken = false;
        if let Some(parent_pid) = parent_pid {
            let parent_tid = self.processes.get(&parent_pid).map(Process::main_tid);
            if let Some(parent) = self.processes.get_mut(&parent_pid) {
                if parent.state == ProcessState::Blocked && parent.waiting_for == Some(pid) {
                    parent.waiting_for = None;
                    if let Some(context) = parent.main_thread_mut().resume_context.as_mut() {
                        context.rax = status;
                    }
                    parent.set_state(ProcessState::Ready);
                    if let Some(parent_tid) = parent_tid {
                        self.enqueue_ready(parent_tid);
                    }
                    parent_woken = true;
                }
            }
        }

        Some(TerminalSignalReport {
            pid,
            name,
            signal,
            status,
            stopped: true,
            parent_woken,
        })
    }

    pub fn continue_user_process(&mut self, caller_pid: Pid, pid: Pid) -> bool {
        let Some(caller_credentials) = self
            .processes
            .get(&caller_pid)
            .map(|process| process.credentials)
        else {
            return false;
        };
        let tid = {
            let Some(process) = self.processes.get_mut(&pid) else {
                return false;
            };
            if caller_credentials.uid != 0 && caller_credentials.uid != process.credentials.uid {
                return false;
            }
            if process.state != ProcessState::Stopped {
                return false;
            }
            process.set_state(ProcessState::Ready);
            process.main_tid()
        };
        self.enqueue_ready(tid);
        true
    }

    pub fn terminate_user_process(
        &mut self,
        caller_pid: Pid,
        target_pid: Pid,
        status: u64,
    ) -> Result<SignalTerminationReport, SignalTerminationError> {
        if caller_pid == target_pid {
            return Err(SignalTerminationError::SelfTarget);
        }

        let caller_credentials = self
            .processes
            .get(&caller_pid)
            .map(|process| process.credentials)
            .ok_or(SignalTerminationError::Missing)?;
        let target = self
            .processes
            .get(&target_pid)
            .ok_or(SignalTerminationError::Missing)?;
        if target.kind != ProcessKind::UserProcess {
            return Err(SignalTerminationError::KernelTask);
        }
        if matches!(
            target.state,
            ProcessState::Exited | ProcessState::Zombie | ProcessState::Reaped
        ) {
            return Err(SignalTerminationError::AlreadyExited);
        }
        if caller_credentials.uid != 0 && caller_credentials.uid != target.credentials.uid {
            return Err(SignalTerminationError::PermissionDenied);
        }

        let name = target.name;
        let parent_pid = target.parent_pid;
        let job_mode = target.job_mode;
        let p4_frame = target.address_space.p4_frame;
        self.mark_exited(target_pid, status);

        let mut parent_woken = false;
        if let Some(parent_pid) = parent_pid {
            let waiting = self.processes.get(&parent_pid).is_some_and(|parent| {
                parent.state == ProcessState::Blocked && parent.waiting_for == Some(target_pid)
            });
            if waiting {
                if let Some(target) = self.processes.get_mut(&target_pid) {
                    target.set_state(ProcessState::Reaped);
                }
                let parent_tid = self.processes.get(&parent_pid).map(Process::main_tid);
                if let Some(parent) = self.processes.get_mut(&parent_pid) {
                    parent.waiting_for = None;
                    parent.sleeping_until_tick = None;
                    if let Some(context) = parent.main_thread_mut().resume_context.as_mut() {
                        context.rax = status;
                    }
                    parent.set_state(ProcessState::Ready);
                }
                if let Some(parent_tid) = parent_tid {
                    self.enqueue_ready(parent_tid);
                }
                parent_woken = true;
            }
        }

        Ok(SignalTerminationReport {
            pid: target_pid,
            name,
            parent_pid,
            job_mode,
            status,
            parent_woken,
            p4_frame,
        })
    }

    pub fn send_signal(
        &mut self,
        caller_pid: Pid,
        target_pid: Pid,
        signal: u64,
    ) -> Result<SignalDelivery, SignalTerminationError> {
        if caller_pid == target_pid {
            return Err(SignalTerminationError::SelfTarget);
        }

        let caller_credentials = self
            .processes
            .get(&caller_pid)
            .map(|process| process.credentials)
            .ok_or(SignalTerminationError::Missing)?;
        let target = self
            .processes
            .get(&target_pid)
            .ok_or(SignalTerminationError::Missing)?;
        if target.kind != ProcessKind::UserProcess {
            return Err(SignalTerminationError::KernelTask);
        }
        if matches!(
            target.state,
            ProcessState::Exited | ProcessState::Zombie | ProcessState::Reaped
        ) {
            return Err(SignalTerminationError::AlreadyExited);
        }
        if caller_credentials.uid != 0 && caller_credentials.uid != target.credentials.uid {
            return Err(SignalTerminationError::PermissionDenied);
        }

        let signal_mask = signal_bit(signal).ok_or(SignalTerminationError::Missing)?;
        if target.blocked_signals & signal_mask != 0 {
            let target = self
                .processes
                .get_mut(&target_pid)
                .expect("validated signal target must remain present");
            target.pending_signals |= signal_mask;
            return Ok(SignalDelivery::Pending {
                pid: target.pid,
                name: target.name,
                signal,
                pending_signals: target.pending_signals,
            });
        }

        let disposition = target.sigterm_disposition;
        if disposition == SIG_IGN {
            return Ok(SignalDelivery::Ignored {
                pid: target.pid,
                name: target.name,
                signal,
            });
        }
        if disposition != SIG_DFL {
            let pid = target.pid;
            let name = target.name;
            if self.activate_signal_handler(target_pid, signal, disposition) {
                return Ok(SignalDelivery::Handled {
                    pid,
                    name,
                    signal,
                    handler: disposition,
                });
            }
            let target = self
                .processes
                .get_mut(&target_pid)
                .expect("validated signal target must remain present");
            target.pending_signals |= signal_mask;
            return Ok(SignalDelivery::Pending {
                pid,
                name,
                signal,
                pending_signals: target.pending_signals,
            });
        }

        self.terminate_user_process(caller_pid, target_pid, 128 + signal)
            .map(SignalDelivery::Terminated)
    }

    fn activate_signal_handler(&mut self, pid: Pid, signal: u64, handler: u64) -> bool {
        let Some(process) = self.processes.get_mut(&pid) else {
            return false;
        };
        let tid = process.main_tid();
        let Some(thread) = process.threads.get_mut(tid) else {
            return false;
        };
        if thread.signal_return_context.is_some() {
            return false;
        }
        let Some(original) = thread.resume_context.take() else {
            return false;
        };
        let mut handler_context = original;
        handler_context.rip = handler;
        handler_context.rdi = signal;
        handler_context.rax = 0;
        thread.signal_return_context = Some(original);
        thread.resume_context = Some(handler_context);
        thread.waiting_for = None;
        thread.state = ThreadState::Ready;
        process.waiting_for = None;
        process.sleeping_until_tick = None;
        process.refresh_state_from_threads();
        self.enqueue_ready(tid);
        true
    }

    pub fn signal_disposition(&self, pid: Pid, signal: u64) -> Option<u64> {
        let process = self.processes.get(&pid)?;
        (signal == SIGTERM).then_some(process.sigterm_disposition)
    }

    pub fn set_signal_disposition(
        &mut self,
        pid: Pid,
        signal: u64,
        disposition: u64,
    ) -> Option<u64> {
        if signal != SIGTERM {
            return None;
        }
        let process = self.processes.get_mut(&pid)?;
        let old = process.sigterm_disposition;
        process.sigterm_disposition = disposition;
        Some(old)
    }

    pub fn activate_running_signal_handler(
        &mut self,
        pid: Pid,
        tid: Tid,
        signal: u64,
        handler: u64,
        original: UserResumeContext,
    ) -> bool {
        let Some(process) = self.processes.get_mut(&pid) else {
            return false;
        };
        let Some(thread) = process.threads.get_mut(tid) else {
            return false;
        };
        if thread.state != ThreadState::Running || thread.signal_return_context.is_some() {
            return false;
        }
        stop_thread_run(thread);
        let mut handler_context = original;
        handler_context.rip = handler;
        handler_context.rdi = signal;
        handler_context.rax = 0;
        thread.signal_return_context = Some(original);
        thread.resume_context = Some(handler_context);
        thread.state = ThreadState::Ready;
        process.refresh_state_from_threads();
        self.enqueue_ready(tid);
        true
    }

    pub fn restore_signal_context(&mut self, pid: Pid, tid: Tid) -> bool {
        let Some(process) = self.processes.get_mut(&pid) else {
            return false;
        };
        let Some(thread) = process.threads.get_mut(tid) else {
            return false;
        };
        if thread.state != ThreadState::Running {
            return false;
        }
        let Some(original) = thread.signal_return_context.take() else {
            return false;
        };
        stop_thread_run(thread);
        thread.resume_context = Some(original);
        thread.state = ThreadState::Ready;
        process.refresh_state_from_threads();
        self.enqueue_ready(tid);
        true
    }

    pub fn update_signal_mask(
        &mut self,
        pid: Pid,
        how: SignalMaskHow,
        mask: u64,
    ) -> Option<SignalMaskUpdate> {
        let process = self.processes.get_mut(&pid)?;
        if process.kind != ProcessKind::UserProcess
            || matches!(
                process.state,
                ProcessState::Exited | ProcessState::Zombie | ProcessState::Reaped
            )
        {
            return None;
        }

        let old_mask = process.blocked_signals;
        process.blocked_signals = match how {
            SignalMaskHow::Block => old_mask | mask,
            SignalMaskHow::Unblock => old_mask & !mask,
            SignalMaskHow::SetMask => mask,
        };
        let deliverable = process.pending_signals & !process.blocked_signals;
        let delivered_signal = lowest_signal(deliverable);
        if let Some(signal) = delivered_signal {
            process.pending_signals &= !signal_bit(signal).unwrap_or(0);
        }

        Some(SignalMaskUpdate {
            old_mask,
            new_mask: process.blocked_signals,
            delivered_signal,
        })
    }

    pub fn pending_signals(&self, pid: Pid) -> Option<u64> {
        self.processes
            .get(&pid)
            .map(|process| process.pending_signals)
    }

    pub fn mark_reaped(&mut self, pid: Pid) -> bool {
        self.remove_from_ready_queue(pid);
        let Some(process) = self.processes.get_mut(&pid) else {
            return false;
        };

        stop_process_run(process);
        process.set_state(ProcessState::Reaped);
        process.exit_status.get_or_insert(0);
        process.waiting_for = None;
        process.sleeping_until_tick = None;
        process.main_thread_mut().resume_context = None;
        true
    }

    pub fn mark_waiting(
        &mut self,
        pid: Pid,
        child_pid: Pid,
        resume_context: UserResumeContext,
    ) -> bool {
        self.remove_from_ready_queue(pid);
        let Some(process) = self.processes.get_mut(&pid) else {
            return false;
        };

        stop_process_run(process);
        process.waiting_for = Some(child_pid);
        process.sleeping_until_tick = None;
        process.main_thread_mut().resume_context = Some(resume_context);
        process.set_state(ProcessState::Blocked);
        true
    }

    pub fn mark_sleeping(
        &mut self,
        pid: Pid,
        wake_tick: u64,
        resume_context: UserResumeContext,
    ) -> bool {
        self.remove_from_ready_queue(pid);
        let Some(process) = self.processes.get_mut(&pid) else {
            return false;
        };
        if process.state != ProcessState::Running {
            return false;
        }

        stop_process_run(process);
        process.waiting_for = None;
        process.sleeping_until_tick = Some(wake_tick);
        process.main_thread_mut().resume_context = Some(resume_context);
        process.set_state(ProcessState::Blocked);
        true
    }

    pub fn mark_yielded(&mut self, pid: Pid, resume_context: UserResumeContext) -> bool {
        let Some(tid) = self.processes.get(&pid).map(Process::main_tid) else {
            return false;
        };
        self.mark_thread_yielded(tid, resume_context)
    }

    pub fn mark_thread_yielded(&mut self, tid: Tid, resume_context: UserResumeContext) -> bool {
        let Some(process) = self.processes.get_by_tid_mut(tid) else {
            return false;
        };
        let Some(thread) = process.threads.get_mut(tid) else {
            return false;
        };
        if thread.state != ThreadState::Running {
            return false;
        }

        stop_thread_run(thread);
        thread.resume_context = Some(resume_context);
        thread.state = ThreadState::Ready;
        process.sleeping_until_tick = None;
        process.refresh_state_from_threads();
        self.enqueue_ready(tid);
        true
    }

    pub fn reap_child(
        &mut self,
        parent_pid: Pid,
        child_pid: Pid,
    ) -> Option<(Pid, &'static str, u64)> {
        let process = self.processes.get_mut(&child_pid)?;
        if process.parent_pid != Some(parent_pid) {
            return None;
        }

        if !matches!(process.state, ProcessState::Exited | ProcessState::Zombie) {
            return None;
        }

        process.set_state(ProcessState::Reaped);
        let result = (process.pid, process.name, process.exit_status.unwrap_or(0));

        if let Some(parent) = self.processes.get_mut(&parent_pid) {
            if parent.waiting_for == Some(child_pid) {
                parent.waiting_for = None;
                parent.main_thread_mut().resume_context = None;
            }
        }

        Some(result)
    }

    pub fn wait_target_status(&self, parent_pid: Pid, child_pid: Pid) -> WaitTargetStatus {
        let Some(process) = self.processes.get(&child_pid) else {
            return WaitTargetStatus::Missing;
        };

        if process.parent_pid != Some(parent_pid) {
            return WaitTargetStatus::NotChild {
                actual_parent: process.parent_pid,
            };
        }

        if !matches!(process.state, ProcessState::Exited | ProcessState::Zombie) {
            return WaitTargetStatus::NotExited {
                state: process.state,
            };
        }

        WaitTargetStatus::Reapable
    }

    pub fn cwd_for(&self, pid: Pid) -> Option<crate::fs::NormalizedPath> {
        self.processes.get(&pid).map(|process| process.cwd)
    }

    pub fn set_cwd(&mut self, pid: Pid, cwd: crate::fs::NormalizedPath) -> bool {
        let Some(process) = self.processes.get_mut(&pid) else {
            return false;
        };
        process.cwd = cwd;
        true
    }

    pub fn resume_waiting_parent(&mut self, parent_pid: Pid, child_pid: Pid) -> Option<WaitResume> {
        let child = self.processes.get(&child_pid)?;
        if child.parent_pid != Some(parent_pid) {
            return None;
        }

        if !matches!(child.state, ProcessState::Exited | ProcessState::Zombie) {
            return None;
        }

        let child_name = child.name;
        let status = child.exit_status.unwrap_or(0);
        let parent = self.processes.get(&parent_pid)?;
        if parent.state != ProcessState::Blocked || parent.waiting_for != Some(child_pid) {
            return None;
        }

        let resume_context = parent.main_thread().resume_context?;
        let p4_frame = parent.address_space.p4_frame;

        if let Some(child) = self.processes.get_mut(&child_pid) {
            child.set_state(ProcessState::Reaped);
        }
        if let Some(parent) = self.processes.get_mut(&parent_pid) {
            parent.waiting_for = None;
            parent.sleeping_until_tick = None;
            parent.main_thread_mut().resume_context = None;
        }
        self.start_process_run(parent_pid);

        Some(WaitResume {
            parent_pid,
            child_pid,
            child_name,
            status,
            resume_context,
            p4_frame,
        })
    }

    pub fn reap_orphan_zombies(&mut self) -> usize {
        let zombie_pids: Vec<Pid> = self
            .processes
            .values()
            .filter(|process| process.state == ProcessState::Zombie)
            .filter(|process| self.should_init_reap(process.parent_pid))
            .map(|process| process.pid)
            .collect();

        let mut reaped = 0usize;
        for pid in zombie_pids {
            if let Some(process) = self.processes.get_mut(&pid) {
                process.set_state(ProcessState::Reaped);
                reaped += 1;
            }
        }

        reaped
    }

    pub fn reap_unwaited_zombies(&mut self) -> usize {
        let zombie_pids: Vec<Pid> = self
            .processes
            .values()
            .filter(|process| process.state == ProcessState::Zombie)
            .filter(|process| process.job_mode == crate::user::program::JobMode::Background)
            .map(|process| process.pid)
            .collect();

        let mut reaped = 0usize;
        for pid in zombie_pids {
            if let Some(process) = self.processes.get_mut(&pid) {
                process.set_state(ProcessState::Reaped);
                reaped += 1;
            }
        }

        reaped
    }

    pub fn wake_sleeping_processes(&mut self, now_tick: u64) -> usize {
        let wake_pids: Vec<Pid> = self
            .processes
            .values()
            .filter(|process| process.state == ProcessState::Blocked)
            .filter(|process| {
                process
                    .sleeping_until_tick
                    .is_some_and(|wake_tick| now_tick >= wake_tick)
            })
            .map(|process| process.pid)
            .collect();

        let mut woken = 0usize;
        for pid in wake_pids {
            let tid = if let Some(process) = self.processes.get_mut(&pid) {
                process.sleeping_until_tick = None;
                process.set_state(ProcessState::Ready);
                Some(process.main_tid())
            } else {
                None
            };
            if let Some(tid) = tid {
                self.enqueue_ready(tid);
                woken += 1;
            }
        }

        woken
    }

    pub fn compact_reaped_history(&mut self, keep: usize) -> usize {
        let reaped_pids: Vec<Pid> = self
            .processes
            .values()
            .filter(|process| process.state == ProcessState::Reaped)
            .map(|process| process.pid)
            .collect();

        let remove_count = reaped_pids.len().saturating_sub(keep);
        for pid in reaped_pids.iter().take(remove_count) {
            self.remove_from_ready_queue(*pid);
            self.processes.remove(pid);
        }

        remove_count
    }

    pub fn dequeue_ready(&mut self) -> Option<Pid> {
        while !self.ready_queue.is_empty() {
            let tid = self.ready_queue.remove(0);
            let Some(process) = self.processes.get_by_tid_mut(tid) else {
                self.ready_queue_skips = self.ready_queue_skips.saturating_add(1);
                continue;
            };
            if process
                .threads
                .get(tid)
                .is_none_or(|thread| thread.state != ThreadState::Ready)
            {
                self.ready_queue_skips = self.ready_queue_skips.saturating_add(1);
                continue;
            }

            let thread = process
                .threads
                .get_mut(tid)
                .expect("TID lookup must resolve inside its owning process");
            thread.state = ThreadState::Running;
            thread.context_switches = thread.context_switches.saturating_add(1);
            thread.last_started_tick = Some(crate::timer::ticks());
            process.refresh_state_from_threads();
            self.user_context_switches += 1;
            return Some(process.pid);
        }

        None
    }

    pub fn schedule_next_ready_user(&mut self) -> Option<ScheduledUserResume> {
        while !self.ready_queue.is_empty() {
            let tid = self.ready_queue.remove(0);
            let Some(process) = self.processes.get_by_tid_mut(tid) else {
                self.ready_queue_skips = self.ready_queue_skips.saturating_add(1);
                continue;
            };
            if process
                .threads
                .get(tid)
                .is_none_or(|thread| thread.state != ThreadState::Ready)
            {
                self.ready_queue_skips = self.ready_queue_skips.saturating_add(1);
                continue;
            }

            let Some(resume_context) = process
                .threads
                .get_mut(tid)
                .and_then(|thread| thread.resume_context.take())
            else {
                self.ready_queue_skips = self.ready_queue_skips.saturating_add(1);
                continue;
            };
            let thread = process
                .threads
                .get_mut(tid)
                .expect("TID lookup must resolve inside its owning process");
            thread.state = ThreadState::Running;
            thread.context_switches = thread.context_switches.saturating_add(1);
            thread.last_started_tick = Some(crate::timer::ticks());
            process.refresh_state_from_threads();
            self.user_context_switches += 1;
            return Some(ScheduledUserResume {
                pid: process.pid,
                tid,
                name: process.name,
                resume_context,
                p4_frame: process.address_space.p4_frame,
            });
        }

        None
    }

    pub fn preempt_running_user(
        &mut self,
        pid: Pid,
        resume_context: UserResumeContext,
    ) -> Option<ScheduledUserResume> {
        let tid = self.processes.get(&pid)?.main_tid();
        self.preempt_running_thread(tid, resume_context)
    }

    pub fn preempt_running_thread(
        &mut self,
        tid: Tid,
        resume_context: UserResumeContext,
    ) -> Option<ScheduledUserResume> {
        if !self.has_ready_thread_except(tid) || !self.mark_thread_yielded(tid, resume_context) {
            return None;
        }

        self.schedule_next_ready_user()
    }

    pub fn record_user_yield(&mut self, pid: Pid) -> bool {
        let Some(tid) = self.processes.get(&pid).map(Process::main_tid) else {
            return false;
        };
        self.record_thread_yield(tid)
    }

    pub fn record_thread_yield(&mut self, tid: Tid) -> bool {
        let Some(process) = self.processes.get_by_tid(tid) else {
            return false;
        };
        if process
            .threads
            .get(tid)
            .is_none_or(|thread| thread.state != ThreadState::Running)
        {
            return false;
        }
        self.user_context_switches += 1;
        true
    }

    pub fn has_ready_process_except(&self, pid: Pid) -> bool {
        let current_tid = self
            .processes
            .get(&pid)
            .map(Process::main_tid)
            .unwrap_or(pid);
        self.has_ready_thread_except(current_tid)
    }

    pub fn has_ready_thread_except(&self, current_tid: Tid) -> bool {
        self.ready_queue.iter().any(|queued_tid| {
            *queued_tid != current_tid
                && self
                    .processes
                    .get_by_tid(*queued_tid)
                    .and_then(|process| process.threads.get(*queued_tid))
                    .is_some_and(|thread| thread.state == ThreadState::Ready)
        })
    }

    pub fn ready_queue_position(&self, pid: Pid) -> Option<usize> {
        let tid = self.processes.get(&pid)?.main_tid();
        self.ready_queue
            .iter()
            .position(|queued_tid| *queued_tid == tid)
            .map(|index| index + 1)
    }

    pub fn contains_pid(&self, pid: Pid) -> bool {
        self.processes.contains_key(&pid)
    }

    pub fn has_active_user_session(&self) -> bool {
        self.processes.values().any(|process| {
            process.kind == ProcessKind::UserProcess
                && !matches!(
                    process.state,
                    ProcessState::Exited | ProcessState::Zombie | ProcessState::Reaped
                )
        })
    }

    fn should_init_reap(&self, parent_pid: Option<Pid>) -> bool {
        let Some(parent_pid) = parent_pid else {
            return false;
        };

        self.processes
            .get(&parent_pid)
            .map(|parent| {
                matches!(
                    parent.state,
                    ProcessState::Exited | ProcessState::Zombie | ProcessState::Reaped
                )
            })
            .unwrap_or(true)
    }

    pub fn mark_isolated(&mut self, pid: Pid) {
        if let Some(process) = self.processes.get_mut(&pid) {
            process.address_space = process.address_space.promoted_isolated_user();
        }
    }

    fn p4_frame_for(&self, pid: Pid) -> Option<u64> {
        self.processes
            .get(&pid)
            .and_then(|process| process.address_space.p4_frame)
    }

    pub fn find_by_entry(&self, entry_point: u64) -> Option<Pid> {
        self.processes
            .values()
            .find(|process| process.address_space.entry_point == entry_point)
            .map(|process| process.pid)
    }

    pub fn stats(&self) -> ProcessStats {
        ProcessStats {
            process_count: self.processes.len(),
            kernel_task_count: self
                .processes
                .values()
                .filter(|process| process.kind == ProcessKind::KernelTask)
                .count(),
            user_process_count: self
                .processes
                .values()
                .filter(|process| process.kind == ProcessKind::UserProcess)
                .count(),
            ready_user_count: self
                .processes
                .values()
                .filter(|process| {
                    process.kind == ProcessKind::UserProcess && process.state == ProcessState::Ready
                })
                .count(),
            running_user_count: self
                .processes
                .values()
                .filter(|process| {
                    process.kind == ProcessKind::UserProcess
                        && process.state == ProcessState::Running
                })
                .count(),
            blocked_user_count: self
                .processes
                .values()
                .filter(|process| {
                    process.kind == ProcessKind::UserProcess
                        && process.state == ProcessState::Blocked
                })
                .count(),
            foreground_user_count: self
                .processes
                .values()
                .filter(|process| {
                    process.kind == ProcessKind::UserProcess
                        && process.job_mode == crate::user::program::JobMode::Foreground
                })
                .count(),
            background_user_count: self
                .processes
                .values()
                .filter(|process| {
                    process.kind == ProcessKind::UserProcess
                        && process.job_mode == crate::user::program::JobMode::Background
                })
                .count(),
            ready_count: self
                .processes
                .values()
                .filter(|process| process.state == ProcessState::Ready)
                .count(),
            ready_queue_count: self.ready_queue.len(),
            running_count: self
                .processes
                .values()
                .filter(|process| process.state == ProcessState::Running)
                .count(),
            exited_count: self
                .processes
                .values()
                .filter(|process| {
                    matches!(
                        process.state,
                        ProcessState::Exited | ProcessState::Zombie | ProcessState::Reaped
                    )
                })
                .count(),
            next_pid: self.next_pid,
            user_context_switches: self.user_context_switches,
            ready_queue_skips: self.ready_queue_skips,
            timer_preempt_checks: TIMER_PREEMPT_CHECKS.load(Ordering::Relaxed),
            timer_preempt_requests: TIMER_PREEMPT_REQUESTS.load(Ordering::Relaxed),
            timer_preempt_yields: TIMER_PREEMPT_YIELDS.load(Ordering::Relaxed),
            timer_preempt_pending: TIMER_PREEMPT_PENDING.load(Ordering::Relaxed),
            thread_count: self
                .processes
                .values()
                .map(|process| process.threads.len())
                .sum(),
        }
    }

    fn enqueue_ready(&mut self, tid: Tid) {
        if !self.ready_queue.contains(&tid) {
            self.ready_queue.push(tid);
        }
    }

    fn remove_from_ready_queue(&mut self, pid: Pid) {
        let tids: Vec<Tid> = self
            .processes
            .get(&pid)
            .map(|process| process.threads.values().map(|thread| thread.tid).collect())
            .unwrap_or_default();
        self.ready_queue.retain(|queued| !tids.contains(queued));
    }

    fn start_process_run(&mut self, pid: Pid) {
        if let Some(process) = self.processes.get_mut(&pid) {
            TIMER_PREEMPT_PENDING.store(false, Ordering::Relaxed);
            CURRENT_USER_PID_ATOMIC.store(pid, Ordering::Relaxed);
            process.set_state(ProcessState::Running);
            let thread = process.main_thread_mut();
            thread.context_switches = thread.context_switches.saturating_add(1);
            thread.last_started_tick = Some(crate::timer::ticks());
            self.user_context_switches += 1;
        }
    }
}

fn take_managed_stack_release(process: &mut Process, tid: Tid) -> Option<(u64, u64)> {
    let p4_frame = process.address_space.p4_frame?;
    let thread = process.threads.get_mut(tid)?;
    if !thread.kernel_managed_stack {
        return None;
    }
    let stack_top = thread.user_stack_top.take()?;
    thread.user_stack_start = None;
    thread.kernel_managed_stack = false;
    Some((p4_frame, stack_top))
}

fn stop_process_run(process: &mut Process) {
    if CURRENT_USER_PID_ATOMIC.load(Ordering::Relaxed) == process.pid {
        CURRENT_USER_PID_ATOMIC.store(0, Ordering::Relaxed);
        CURRENT_USER_TID_ATOMIC.store(0, Ordering::Relaxed);
        TIMER_PREEMPT_PENDING.store(false, Ordering::Relaxed);
    }
    stop_thread_run(process.main_thread_mut());
}

fn stop_thread_run(thread: &mut Thread) {
    if let Some(started_at) = thread.last_started_tick.take() {
        let elapsed = crate::timer::ticks().saturating_sub(started_at);
        thread.runtime_ticks = thread.runtime_ticks.saturating_add(elapsed);
    }
}

fn runtime_ticks_snapshot(process: &Process, now: u64) -> u64 {
    let active_elapsed = match (process.state, process.main_thread().last_started_tick) {
        (ProcessState::Running, Some(started_at)) => now.saturating_sub(started_at),
        _ => 0,
    };
    process
        .main_thread()
        .runtime_ticks
        .saturating_add(active_elapsed)
}

lazy_static! {
    pub static ref PROCESS_TABLE: Mutex<ProcessTable> = Mutex::new(ProcessTable::new());
    static ref CURRENT_USER_PROCESS: Mutex<Option<Pid>> = Mutex::new(None);
    static ref CURRENT_USER_THREAD: Mutex<Option<Tid>> = Mutex::new(None);
    static ref LAST_EXITED_PROCESS: Mutex<Option<Pid>> = Mutex::new(None);
}

fn set_current_user(pid: Option<Pid>, tid: Option<Tid>) {
    *CURRENT_USER_PROCESS.lock() = pid;
    *CURRENT_USER_THREAD.lock() = tid;
    CURRENT_USER_PID_ATOMIC.store(pid.unwrap_or(0), Ordering::Relaxed);
    CURRENT_USER_TID_ATOMIC.store(tid.unwrap_or(0), Ordering::Relaxed);
    if pid.is_none() {
        TIMER_PREEMPT_PENDING.store(false, Ordering::Relaxed);
    }
}

fn take_current_user() -> Option<(Pid, Tid)> {
    let pid = CURRENT_USER_PROCESS.lock().take();
    let tid = CURRENT_USER_THREAD.lock().take();
    CURRENT_USER_PID_ATOMIC.store(0, Ordering::Relaxed);
    CURRENT_USER_TID_ATOMIC.store(0, Ordering::Relaxed);
    TIMER_PREEMPT_PENDING.store(false, Ordering::Relaxed);
    Some((pid?, tid?))
}

pub fn init_process_table() {
    let mut table = PROCESS_TABLE.lock();
    if table.stats().process_count == 0 {
        let pid = table.create_reserved_process(
            0,
            None,
            "kernel",
            "kernel",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0),
        );
        table.mark_ready(pid);
        crate::serial_println!("[PROCESS] Created kernel process pid={}", pid);
    }
}

pub fn stats() -> ProcessStats {
    PROCESS_TABLE.lock().stats()
}

pub fn create_kernel_thread() -> Option<Tid> {
    PROCESS_TABLE.lock().create_kernel_thread()
}

pub fn set_kernel_thread_state(tid: Tid, state: ThreadState) -> bool {
    PROCESS_TABLE.lock().set_kernel_thread_state(tid, state)
}

pub fn kernel_thread_state(tid: Tid) -> Option<ThreadState> {
    PROCESS_TABLE.lock().kernel_thread_state(tid)
}

pub fn kernel_thread_runtime_ticks(tid: Tid) -> Option<u64> {
    PROCESS_TABLE.lock().kernel_thread_runtime_ticks(tid)
}

pub fn userland_owns_keyboard() -> bool {
    crate::user::program::has_pending() || PROCESS_TABLE.lock().has_active_user_session()
}

pub fn record_timer_preemption_check() {
    TIMER_PREEMPT_CHECKS.fetch_add(1, Ordering::Relaxed);
    if CURRENT_USER_PID_ATOMIC.load(Ordering::Relaxed) == 0 {
        return;
    }

    let tick = crate::timer::ticks();
    let last = LAST_PREEMPT_REQUEST_TICK.load(Ordering::Relaxed);
    if tick.saturating_sub(last) < USER_PREEMPT_QUANTUM_TICKS {
        return;
    }

    if LAST_PREEMPT_REQUEST_TICK
        .compare_exchange(last, tick, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        TIMER_PREEMPT_REQUESTS.fetch_add(1, Ordering::Relaxed);
        TIMER_PREEMPT_PENDING.store(true, Ordering::Relaxed);
    }
}

pub fn timer_preempt_quantum_ticks() -> u64 {
    USER_PREEMPT_QUANTUM_TICKS
}

pub fn consume_timer_preempt_request_for_yield() -> bool {
    if TIMER_PREEMPT_PENDING
        .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        TIMER_PREEMPT_YIELDS.fetch_add(1, Ordering::Relaxed);
        return true;
    }

    false
}

pub fn preempt_current_user(resume_context: UserResumeContext) -> Option<ScheduledUserResume> {
    if !TIMER_PREEMPT_PENDING.load(Ordering::Relaxed) {
        return None;
    }

    let tid = CURRENT_USER_TID_ATOMIC.load(Ordering::Relaxed);
    if tid == 0 {
        return None;
    }

    let mut table = PROCESS_TABLE.lock();
    let resume = table.preempt_running_thread(tid, resume_context)?;
    drop(table);

    *CURRENT_USER_PROCESS.lock() = Some(resume.pid);
    *CURRENT_USER_THREAD.lock() = Some(resume.tid);
    CURRENT_USER_PID_ATOMIC.store(resume.pid, Ordering::Relaxed);
    CURRENT_USER_TID_ATOMIC.store(resume.tid, Ordering::Relaxed);
    TIMER_PREEMPT_PENDING.store(false, Ordering::Relaxed);
    TIMER_PREEMPT_YIELDS.fetch_add(1, Ordering::Relaxed);
    Some(resume)
}

pub fn record_user_yield(pid: Pid) -> bool {
    PROCESS_TABLE.lock().record_user_yield(pid)
}

pub fn record_current_thread_yield() -> bool {
    let Some(tid) = current_user_tid() else {
        return false;
    };
    PROCESS_TABLE.lock().record_thread_yield(tid)
}

pub fn has_ready_process_except(pid: Pid) -> bool {
    PROCESS_TABLE.lock().has_ready_process_except(pid)
}

pub fn has_ready_thread_except_current() -> bool {
    let Some(tid) = current_user_tid() else {
        return false;
    };
    PROCESS_TABLE.lock().has_ready_thread_except(tid)
}

pub fn create_current_user_thread(
    entry: u64,
    stack_start: u64,
    stack_top: u64,
    initial_rsp: u64,
    arg: u64,
) -> Option<Tid> {
    let pid = current_user_pid()?;
    PROCESS_TABLE
        .lock()
        .create_user_thread(pid, entry, stack_start, stack_top, initial_rsp, arg)
}

pub fn create_current_managed_user_thread(entry: u64, arg: u64) -> Option<Tid> {
    let pid = current_user_pid()?;
    PROCESS_TABLE
        .lock()
        .create_managed_user_thread(pid, entry, arg)
}

pub fn mark_current_thread_exited() -> Option<(Pid, Tid)> {
    let (pid, tid) = take_current_user()?;
    if PROCESS_TABLE.lock().mark_thread_exited(tid).is_none() {
        set_current_user(Some(pid), Some(tid));
        return None;
    }
    Some((pid, tid))
}

pub fn join_current_thread(target_tid: Tid, resume_context: UserResumeContext) -> ThreadJoinStatus {
    let Some(caller_tid) = current_user_tid() else {
        return ThreadJoinStatus::Missing;
    };
    PROCESS_TABLE
        .lock()
        .join_thread(caller_tid, target_tid, resume_context)
}

pub fn block_current_thread(resume_context: UserResumeContext) -> Option<Tid> {
    let tid = current_user_tid()?;
    PROCESS_TABLE
        .lock()
        .block_thread(tid, resume_context)
        .then_some(tid)
}

pub fn wake_thread(tid: Tid, return_value: u64) -> bool {
    PROCESS_TABLE.lock().wake_thread(tid, return_value)
}

pub fn reserve_pid() -> Pid {
    PROCESS_TABLE.lock().reserve_pid()
}

pub fn register_exec(
    program: crate::user::program::UserProgram,
    loaded: crate::user::loader::LoadedProgram,
) -> ExecContext {
    let mut table = PROCESS_TABLE.lock();
    let program_size = loaded.image_len as u64;
    let mut layout =
        crate::user::ring3::memory_layout_for_pid_with_program_size(program.pid, program_size);
    let address_space = match crate::user::address_space::take_prepared_p4_frame() {
        Some(p4_frame) => {
            match unsafe { crate::user::address_space::smoke_switch_to_p4_frame(p4_frame) } {
                Ok(frame) => {
                    crate::serial_println!("[PROCESS] verified prepared P4 frame {:#x}", frame);
                    AddressSpace::verified_prepared_user(loaded.entry.as_u64(), layout, p4_frame)
                }
                Err(err) => {
                    crate::serial_println!(
                        "[PROCESS] prepared P4 frame {:#x} smoke verification skipped: {:?}",
                        p4_frame,
                        err
                    );
                    AddressSpace::verified_prepared_user(loaded.entry.as_u64(), layout, p4_frame)
                }
            }
        }
        None => {
            layout.stack_top = crate::user::ring3::FIRST_USER_STACK_TOP;
            layout.stack_start =
                crate::user::ring3::FIRST_USER_STACK_TOP - crate::user::ring3::USER_STACK_SIZE;
            AddressSpace::kernel_shared_user(loaded.entry.as_u64(), layout)
        }
    };
    let address_space = if !address_space.p4_verified && address_space.p4_frame.is_none() {
        layout.stack_top = crate::user::ring3::FIRST_USER_STACK_TOP;
        layout.stack_start =
            crate::user::ring3::FIRST_USER_STACK_TOP - crate::user::ring3::USER_STACK_SIZE;
        match address_space.p4_frame {
            Some(p4_frame) => AddressSpace::prepared_user(loaded.entry.as_u64(), layout, p4_frame),
            None => AddressSpace::kernel_shared_user(loaded.entry.as_u64(), layout),
        }
    } else {
        address_space
    };
    let initial_stack =
        crate::user::ring3::build_user_initial_stack(layout, program.path, program.arg.as_bytes())
            .map(|stack| stack.stack_pointer)
            .unwrap_or(layout.stack_top);

    let pid = table.create_reserved_process_with_job_mode(
        program.pid,
        program.parent_pid,
        program.name,
        program.path,
        program.arg,
        program.job_mode,
        address_space,
    );
    table.mark_running_fresh(pid);
    let tid = table
        .processes
        .get(&pid)
        .map(Process::main_tid)
        .expect("registered process must have a main thread");
    set_current_user(Some(pid), Some(tid));
    crate::serial_println!(
        "[PROCESS] exec pid={} entry={:#x} stack={:#x} stack_top={:#x} arg_len={}",
        pid,
        loaded.entry.as_u64(),
        initial_stack,
        layout.stack_top,
        program.arg.len
    );
    ExecContext {
        pid,
        entry_point: loaded.entry.as_u64(),
        user_stack_top: initial_stack,
        program_path: program.path,
        arg: program.arg,
        layout,
        p4_frame: address_space.p4_frame,
        p4_verified: address_space.p4_verified,
    }
}

pub fn mark_current_user_exited(status: u64) -> Option<Pid> {
    let (pid, _) = take_current_user()?;
    crate::user::syscall::close_process_files(pid);
    let mut table = PROCESS_TABLE.lock();
    let p4_frame = table.p4_frame_for(pid);
    table.mark_exited(pid, status);
    *LAST_EXITED_PROCESS.lock() = Some(pid);
    drop(table);
    if let Some(p4_frame) = p4_frame {
        crate::user::address_space::release_prepared_p4_frame(p4_frame);
    }
    Some(pid)
}

pub fn checkout_current_user_if_blocked() -> Option<BlockedUserReport> {
    let pid = *CURRENT_USER_PROCESS.lock();
    let pid = pid?;
    let tid = *CURRENT_USER_THREAD.lock();
    let tid = tid?;
    let table = PROCESS_TABLE.lock();
    let process = table.processes.get(&pid)?;
    let thread = process.threads.get(tid)?;
    if thread.state != ThreadState::Blocked {
        return None;
    }

    let report = BlockedUserReport {
        pid,
        tid,
        name: process.name,
        waiting_for: process.waiting_for,
        waiting_for_thread: thread.waiting_for,
        resume_context: thread.resume_context,
    };
    drop(table);
    set_current_user(None, None);
    Some(report)
}

pub fn checkout_current_user_if_yielded(
    resume_context: UserResumeContext,
) -> Option<YieldedUserReport> {
    let (pid, tid) = take_current_user()?;
    let mut table = PROCESS_TABLE.lock();
    if !table.mark_thread_yielded(tid, resume_context) {
        set_current_user(Some(pid), Some(tid));
        return None;
    }

    let process = table.processes.get(&pid)?;
    Some(YieldedUserReport {
        pid,
        name: process.name,
        resume_context,
    })
}

pub fn schedule_next_ready_user() -> Option<ScheduledUserResume> {
    let mut table = PROCESS_TABLE.lock();
    let resume = table.schedule_next_ready_user()?;
    set_current_user(Some(resume.pid), Some(resume.tid));
    Some(resume)
}

pub fn mark_process_isolated(pid: Pid) {
    PROCESS_TABLE.lock().mark_isolated(pid);
}

pub fn mark_waiting(pid: Pid, child_pid: Pid, resume_context: UserResumeContext) -> bool {
    PROCESS_TABLE
        .lock()
        .mark_waiting(pid, child_pid, resume_context)
}

pub fn mark_current_user_sleeping(wake_tick: u64, resume_context: UserResumeContext) -> bool {
    let pid = match *CURRENT_USER_PROCESS.lock() {
        Some(pid) => pid,
        None => return false,
    };
    PROCESS_TABLE
        .lock()
        .mark_sleeping(pid, wake_tick, resume_context)
}

pub fn wake_sleeping_processes(now_tick: u64) -> usize {
    PROCESS_TABLE.lock().wake_sleeping_processes(now_tick)
}

pub fn reap_child(parent_pid: Pid, child_pid: Pid) -> Option<(Pid, &'static str, u64)> {
    PROCESS_TABLE.lock().reap_child(parent_pid, child_pid)
}

pub fn wait_target_status(parent_pid: Pid, child_pid: Pid) -> WaitTargetStatus {
    PROCESS_TABLE
        .lock()
        .wait_target_status(parent_pid, child_pid)
}

pub fn contains_pid(pid: Pid) -> bool {
    PROCESS_TABLE.lock().contains_pid(pid)
}

pub fn terminate_user_process(
    caller_pid: Pid,
    target_pid: Pid,
    status: u64,
) -> Result<SignalTerminationReport, SignalTerminationError> {
    let report = PROCESS_TABLE
        .lock()
        .terminate_user_process(caller_pid, target_pid, status)?;
    crate::user::syscall::close_process_files(target_pid);
    if let Some(p4_frame) = report.p4_frame {
        crate::user::address_space::release_prepared_p4_frame(p4_frame);
    }
    Ok(report)
}

pub fn send_signal(
    caller_pid: Pid,
    target_pid: Pid,
    signal: u64,
) -> Result<SignalDelivery, SignalTerminationError> {
    let delivery = PROCESS_TABLE
        .lock()
        .send_signal(caller_pid, target_pid, signal)?;
    if let SignalDelivery::Terminated(report) = delivery {
        crate::user::syscall::close_process_files(target_pid);
        if let Some(p4_frame) = report.p4_frame {
            crate::user::address_space::release_prepared_p4_frame(p4_frame);
        }
    }
    Ok(delivery)
}

pub fn update_current_signal_mask(how: SignalMaskHow, mask: u64) -> Option<SignalMaskUpdate> {
    let pid = current_user_pid()?;
    PROCESS_TABLE.lock().update_signal_mask(pid, how, mask)
}

pub fn current_pending_signals() -> Option<u64> {
    let pid = current_user_pid()?;
    PROCESS_TABLE.lock().pending_signals(pid)
}

pub fn current_signal_disposition(signal: u64) -> Option<u64> {
    let pid = current_user_pid()?;
    PROCESS_TABLE.lock().signal_disposition(pid, signal)
}

pub fn set_current_signal_disposition(signal: u64, disposition: u64) -> Option<u64> {
    let pid = current_user_pid()?;
    PROCESS_TABLE
        .lock()
        .set_signal_disposition(pid, signal, disposition)
}

pub fn checkout_current_user_for_signal_handler(
    signal: u64,
    handler: u64,
    original: UserResumeContext,
) -> Option<SignalHandlerReport> {
    let pid = current_user_pid()?;
    let tid = current_user_tid()?;
    if !PROCESS_TABLE
        .lock()
        .activate_running_signal_handler(pid, tid, signal, handler, original)
    {
        return None;
    }
    take_current_user()?;
    Some(SignalHandlerReport {
        pid,
        tid,
        signal,
        handler,
    })
}

pub fn checkout_current_user_after_sigreturn() -> Option<(Pid, Tid)> {
    let pid = current_user_pid()?;
    let tid = current_user_tid()?;
    if !PROCESS_TABLE.lock().restore_signal_context(pid, tid) {
        return None;
    }
    take_current_user()
}

pub fn resume_waiting_parent(parent_pid: Pid, child_pid: Pid) -> Option<WaitResume> {
    let mut table = PROCESS_TABLE.lock();
    let resume = table.resume_waiting_parent(parent_pid, child_pid)?;
    let tid = table.processes.get(&parent_pid).map(Process::main_tid);
    drop(table);
    set_current_user(Some(parent_pid), tid);
    Some(resume)
}

pub fn mark_reaped(pid: Pid) -> bool {
    PROCESS_TABLE.lock().mark_reaped(pid)
}

pub fn reap_orphan_zombies() -> usize {
    PROCESS_TABLE.lock().reap_orphan_zombies()
}

pub fn reap_unwaited_zombies() -> usize {
    PROCESS_TABLE.lock().reap_unwaited_zombies()
}

pub fn compact_reaped_history() -> usize {
    PROCESS_TABLE
        .lock()
        .compact_reaped_history(DEFAULT_REAPED_HISTORY_LIMIT)
}

pub fn mark_current_user_fault(addr: u64, rip: u64, status: u64) -> Option<UserFaultReport> {
    let (pid, _) = take_current_user()?;
    crate::user::syscall::close_process_files(pid);
    let mut table = PROCESS_TABLE.lock();
    let process = table.processes.get(&pid)?;
    let kind = classify_user_fault(addr, process.address_space.layout);
    let report = UserFaultReport {
        pid,
        name: process.name,
        addr,
        rip,
        kind,
        status,
    };
    let p4_frame = process.address_space.p4_frame;
    table.mark_exited(pid, status);
    *LAST_EXITED_PROCESS.lock() = Some(pid);
    drop(table);
    if let Some(p4_frame) = p4_frame {
        crate::user::address_space::release_prepared_p4_frame(p4_frame);
    }
    Some(report)
}

pub fn mark_current_user_exception(rip: u64, status: u64) -> Option<UserExceptionReport> {
    let (pid, _) = take_current_user()?;
    crate::user::syscall::close_process_files(pid);
    let mut table = PROCESS_TABLE.lock();
    let process = table.processes.get(&pid)?;
    let report = UserExceptionReport {
        pid,
        name: process.name,
        rip,
        status,
    };
    let p4_frame = process.address_space.p4_frame;
    table.mark_exited(pid, status);
    *LAST_EXITED_PROCESS.lock() = Some(pid);
    drop(table);
    if let Some(p4_frame) = p4_frame {
        crate::user::address_space::release_prepared_p4_frame(p4_frame);
    }
    Some(report)
}

pub fn classify_user_fault(
    addr: u64,
    layout: crate::user::ring3::UserMemoryLayout,
) -> UserFaultKind {
    if addr == 0 {
        return UserFaultKind::Null;
    }

    if addr >= 0xffff_8000_0000_0000 {
        return UserFaultKind::KernelSpace;
    }

    let guard_start = layout
        .stack_start
        .saturating_sub(crate::user::ring3::USER_STACK_GUARD_SIZE);
    if addr >= guard_start && addr < layout.stack_start {
        return UserFaultKind::StackGuard;
    }

    let in_program_area = addr >= layout.program_start && addr < layout.program_end;
    let in_stack_area = addr >= layout.stack_start && addr < layout.stack_top;
    if in_program_area || in_stack_area {
        return UserFaultKind::UserRange;
    }

    UserFaultKind::OutsideUserRange
}

pub fn current_user_pid() -> Option<Pid> {
    *CURRENT_USER_PROCESS.lock()
}

pub fn current_user_tid() -> Option<Tid> {
    *CURRENT_USER_THREAD.lock()
}

pub fn current_user_name() -> Option<&'static str> {
    let pid = current_user_pid()?;
    let table = PROCESS_TABLE.lock();
    table.processes.get(&pid).map(|process| process.name)
}

pub fn current_user_credentials() -> Option<ProcessCredentials> {
    let pid = current_user_pid()?;
    PROCESS_TABLE.lock().credentials_for(pid)
}

pub fn current_process_group_id() -> Option<Pid> {
    let pid = current_user_pid()?;
    PROCESS_TABLE.lock().process_group_id(pid)
}

pub fn set_terminal_foreground_job(pid: Pid) {
    TERMINAL_FOREGROUND_PID.store(pid, Ordering::Release);
}

pub fn clear_terminal_foreground_job(pid: Pid) {
    let _ = TERMINAL_FOREGROUND_PID.compare_exchange(pid, 0, Ordering::AcqRel, Ordering::Acquire);
}

pub fn terminal_foreground_job() -> Option<Pid> {
    let pid = TERMINAL_FOREGROUND_PID.load(Ordering::Acquire);
    (pid != 0).then_some(pid)
}

pub fn deliver_terminal_signal(signal: u64) -> Option<TerminalSignalReport> {
    let pid = terminal_foreground_job()?;
    match signal {
        SIGINT => {
            let caller_pid = PROCESS_TABLE
                .lock()
                .processes
                .get(&pid)
                .and_then(|process| process.parent_pid)
                .unwrap_or(1);
            let report = terminate_user_process(caller_pid, pid, 128 + SIGINT).ok()?;
            clear_terminal_foreground_job(pid);
            crate::user::program::take_user_shell_wait_request_for_child(pid);
            Some(TerminalSignalReport {
                pid,
                name: report.name,
                signal,
                status: 128 + signal,
                stopped: false,
                parent_woken: report.parent_woken,
            })
        }
        SIGTSTP => {
            let report = PROCESS_TABLE.lock().stop_user_process(pid, signal)?;
            clear_terminal_foreground_job(pid);
            crate::user::program::take_user_shell_wait_request_for_child(pid);
            Some(report)
        }
        _ => None,
    }
}

pub fn continue_process(caller_pid: Pid, pid: Pid) -> bool {
    PROCESS_TABLE.lock().continue_user_process(caller_pid, pid)
}

pub fn process_group_id(pid: Pid) -> Option<Pid> {
    PROCESS_TABLE.lock().process_group_id(pid)
}

pub fn session_id(pid: Pid) -> Option<Pid> {
    PROCESS_TABLE.lock().session_id(pid)
}

pub fn set_process_group(
    caller_pid: Pid,
    target_pid: Pid,
    process_group_id: Pid,
) -> Result<Pid, ProcessGroupError> {
    PROCESS_TABLE
        .lock()
        .set_process_group(caller_pid, target_pid, process_group_id)
}

pub fn create_current_session() -> Result<Pid, ProcessGroupError> {
    let pid = current_user_pid().ok_or(ProcessGroupError::Missing)?;
    PROCESS_TABLE.lock().create_session(pid)
}

pub fn set_current_user_credentials(username: &str) -> bool {
    let Some(pid) = current_user_pid() else {
        return false;
    };
    PROCESS_TABLE
        .lock()
        .set_credentials(pid, ProcessCredentials::from_username(username))
}

pub fn current_working_directory() -> Option<crate::fs::NormalizedPath> {
    let pid = current_user_pid()?;
    PROCESS_TABLE.lock().cwd_for(pid)
}

pub fn change_current_working_directory(
    path: &str,
) -> Result<crate::fs::NormalizedPath, crate::fs::FsError> {
    let pid = current_user_pid().ok_or(crate::fs::FsError::NotFound)?;
    let cwd = PROCESS_TABLE
        .lock()
        .cwd_for(pid)
        .unwrap_or_else(crate::fs::NormalizedPath::root);
    let next = crate::fs::normalize_path_from(cwd.as_str(), path)?;
    let stat = crate::fs::stat(next.as_str())?;
    if stat.file_type != crate::fs::FileType::Directory {
        return Err(crate::fs::FsError::NotDirectory);
    }

    PROCESS_TABLE.lock().set_cwd(pid, next);
    Ok(next)
}

pub fn current_user_layout() -> Option<crate::user::ring3::UserMemoryLayout> {
    let pid = current_user_pid()?;
    let table = PROCESS_TABLE.lock();
    table
        .processes
        .get(&pid)
        .map(|process| process.address_space.layout)
}

pub fn current_user_range_owned(ptr: u64, end: u64) -> bool {
    let Some(pid) = current_user_pid() else {
        return false;
    };
    let table = PROCESS_TABLE.lock();
    let Some(process) = table.processes.get(&pid) else {
        return false;
    };
    let layout = process.address_space.layout;
    let in_program = ptr >= layout.program_start && end <= layout.program_end;
    in_program
        || process.threads.values().any(|thread| {
            thread
                .user_stack_start
                .zip(thread.user_stack_top)
                .is_some_and(|(start, top)| ptr >= start && end <= top)
        })
}

pub fn current_user_address_space() -> Option<AddressSpace> {
    let pid = current_user_pid()?;
    let table = PROCESS_TABLE.lock();
    table
        .processes
        .get(&pid)
        .map(|process| process.address_space)
}

pub fn last_exit_summary() -> Option<(Pid, &'static str, u64)> {
    let pid = (*LAST_EXITED_PROCESS.lock())?;
    let table = PROCESS_TABLE.lock();
    let process = table.processes.get(&pid)?;
    Some((pid, process.name, process.exit_status.unwrap_or(0)))
}

pub fn print_processes() {
    let table = PROCESS_TABLE.lock();
    let stats = table.stats();
    crate::println!(
        "processes count={} threads={} kernel={} user={} fg={} bg={} uready={} urun={} ublock={} ready={} rq={} rqskip={} running={} exited={} next_pid={} syscall_traps={} user_ctx={} preempt_checks={} preempt_req={} preempt_yields={} preempt_pending={} quantum={}",
        stats.process_count,
        stats.thread_count,
        stats.kernel_task_count,
        stats.user_process_count,
        stats.foreground_user_count,
        stats.background_user_count,
        stats.ready_user_count,
        stats.running_user_count,
        stats.blocked_user_count,
        stats.ready_count,
        stats.ready_queue_count,
        stats.ready_queue_skips,
        stats.running_count,
        stats.exited_count,
        stats.next_pid,
        crate::user::syscall::trap_count(),
        stats.user_context_switches,
        stats.timer_preempt_checks,
        stats.timer_preempt_requests,
        stats.timer_preempt_yields,
        stats.timer_preempt_pending as u8,
        USER_PREEMPT_QUANTUM_TICKS
    );

    for process in table.processes.values() {
        crate::println!(
            "pid={} tid={} ppid={:?} pgid={} sid={} kind={:?} mode={:?} name={} state={:?} tstate={:?} status={:?} wait={:?} ctx={} ticks={} resume={:?} as={:?} p4={:?} p4ok={} entry={:#x} mem={:#x}-{:#x} stack={:#x}-{:#x} arg={}",
            process.pid,
            process.main_tid(),
            process.parent_pid,
            process.process_group_id,
            process.session_id,
            process.kind,
            process.job_mode,
            process.name,
            process.state,
            process.main_thread().state,
            process.exit_status,
            process.waiting_for,
            process.main_thread().context_switches,
            runtime_ticks_snapshot(process, crate::timer::ticks()),
            process.main_thread().resume_context,
            process.address_space.kind,
            process.address_space.p4_frame,
            process.address_space.p4_verified as u8,
            process.address_space.entry_point,
            process.address_space.layout.program_start,
            process.address_space.layout.program_end,
            process.address_space.layout.stack_start,
            process.address_space.layout.stack_top,
            process.arg.as_str()
        );
    }
}

pub fn write_processes_to_buffer(
    out: &mut [u8],
    syscall_traps: u64,
    user_yields: u64,
    pending_programs: usize,
) -> usize {
    let table = PROCESS_TABLE.lock();
    let stats = table.stats();
    let mut writer = BufferWriter::new(out);

    writer.write_str("procs total=");
    writer.write_usize(stats.process_count);
    writer.write_str(" threads=");
    writer.write_usize(stats.thread_count);
    writer.write_str(" kern=");
    writer.write_usize(stats.kernel_task_count);
    writer.write_str(" user=");
    writer.write_usize(stats.user_process_count);
    writer.write_str(" fg=");
    writer.write_usize(stats.foreground_user_count);
    writer.write_str(" bg=");
    writer.write_usize(stats.background_user_count);
    writer.write_str(" uready=");
    writer.write_usize(stats.ready_user_count);
    writer.write_str(" urun=");
    writer.write_usize(stats.running_user_count);
    writer.write_str(" ublock=");
    writer.write_usize(stats.blocked_user_count);
    writer.write_str(" ready=");
    writer.write_usize(stats.ready_count);
    writer.write_str(" rq=");
    writer.write_usize(stats.ready_queue_count);
    writer.write_str(" rqskip=");
    writer.write_u64(stats.ready_queue_skips);
    writer.write_str(" running=");
    writer.write_usize(stats.running_count);
    writer.write_str(" done=");
    writer.write_usize(stats.exited_count);
    writer.write_str(" next=");
    writer.write_u32(stats.next_pid);
    writer.write_str(" pending=");
    writer.write_usize(pending_programs);
    writer.write_str(" traps=");
    writer.write_u64(syscall_traps);
    writer.write_str(" yields=");
    writer.write_u64(user_yields);
    writer.write_str(" ctx=");
    writer.write_u64(stats.user_context_switches);
    writer.write_str(" preempt=");
    writer.write_u64(stats.timer_preempt_checks);
    writer.write_str(" preq=");
    writer.write_u64(stats.timer_preempt_requests);
    writer.write_str(" py=");
    writer.write_u64(stats.timer_preempt_yields);
    writer.write_str(" pp=");
    writer.write_u64(stats.timer_preempt_pending as u64);
    writer.write_str(" q=");
    writer.write_u64(USER_PREEMPT_QUANTUM_TICKS);
    writer.write_byte(b'\n');
    writer.write_str(
        "PID  TID  THR PPID PGID SID  KIND MODE STATE   EXIT WAIT RQ  CTX   TICKS NAME       ARG\n",
    );
    let now = crate::timer::ticks();

    for process in table.processes.values() {
        writer.write_u32_padded(process.pid, 4);
        writer.write_byte(b' ');
        writer.write_u32_padded(process.main_tid(), 4);
        writer.write_byte(b' ');
        writer.write_usize_padded(process.threads.len(), 3);
        writer.write_byte(b' ');
        writer.write_opt_u32_padded(process.parent_pid, 4);
        writer.write_byte(b' ');
        writer.write_u32_padded(process.process_group_id, 4);
        writer.write_byte(b' ');
        writer.write_u32_padded(process.session_id, 4);
        writer.write_byte(b' ');
        writer.write_str(process_kind_name(process.kind));
        writer.write_byte(b' ');
        writer.write_str(job_mode_name(process.job_mode));
        writer.write_byte(b' ');
        writer.write_str(process_state_name(process.state));
        writer.pad_to_width(process_state_name(process.state).len(), 7);
        writer.write_byte(b' ');
        writer.write_exit_status(process.exit_status);
        writer.write_byte(b' ');
        writer.write_opt_u32_padded(process.waiting_for, 4);
        writer.write_byte(b' ');
        writer.write_opt_usize_padded(table.ready_queue_position(process.pid), 3);
        writer.write_byte(b' ');
        writer.write_u64_padded(process.main_thread().context_switches, 5);
        writer.write_byte(b' ');
        writer.write_u64_padded(runtime_ticks_snapshot(process, now), 5);
        writer.write_byte(b' ');
        writer.write_str_padded(process.name, 10);
        writer.write_str(process.arg.as_str());
        writer.write_byte(b'\n');
    }

    writer.len()
}

fn process_state_name(state: ProcessState) -> &'static str {
    match state {
        ProcessState::Created => "Created",
        ProcessState::Ready => "Ready",
        ProcessState::Running => "Running",
        ProcessState::Blocked => "Blocked",
        ProcessState::Stopped => "Stopped",
        ProcessState::Exited => "Exited",
        ProcessState::Zombie => "Zombie",
        ProcessState::Reaped => "Reaped",
    }
}

fn process_kind_name(kind: ProcessKind) -> &'static str {
    match kind {
        ProcessKind::KernelTask => "KERN",
        ProcessKind::UserProcess => "USER",
    }
}

fn job_mode_name(mode: crate::user::program::JobMode) -> &'static str {
    match mode {
        crate::user::program::JobMode::Foreground => "FG",
        crate::user::program::JobMode::Background => "BG",
    }
}

struct BufferWriter<'a> {
    out: &'a mut [u8],
    len: usize,
}

impl<'a> BufferWriter<'a> {
    fn new(out: &'a mut [u8]) -> Self {
        Self { out, len: 0 }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn write_byte(&mut self, byte: u8) {
        if self.len < self.out.len() {
            self.out[self.len] = byte;
        }
        self.len = self.len.saturating_add(1);
    }

    fn write_str(&mut self, text: &str) {
        for &byte in text.as_bytes() {
            self.write_byte(byte);
        }
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_usize_padded(&mut self, value: usize, width: usize) {
        let digits = decimal_len(value as u64);
        self.write_usize(value);
        self.pad_to_width(digits, width);
    }

    fn write_u32(&mut self, value: u32) {
        self.write_u64(value as u64);
    }

    fn write_u32_padded(&mut self, value: u32, width: usize) {
        let digits = decimal_len(value as u64);
        self.pad_to_width(digits, width);
        self.write_u32(value);
    }

    fn write_u64_padded(&mut self, value: u64, width: usize) {
        let digits = decimal_len(value);
        self.pad_to_width(digits, width);
        self.write_u64(value);
    }

    fn write_opt_u32_padded(&mut self, value: Option<u32>, width: usize) {
        match value {
            Some(value) => self.write_u32_padded(value, width),
            None => {
                self.pad_to_width(1, width);
                self.write_byte(b'-');
            }
        }
    }

    fn write_opt_usize_padded(&mut self, value: Option<usize>, width: usize) {
        match value {
            Some(value) => self.write_u64_padded(value as u64, width),
            None => {
                self.pad_to_width(1, width);
                self.write_byte(b'-');
            }
        }
    }

    fn write_exit_status(&mut self, value: Option<u64>) {
        match value {
            Some(value) => {
                let digits = decimal_len(value);
                self.pad_to_width(digits, 4);
                self.write_u64(value);
            }
            None => {
                self.pad_to_width(1, 4);
                self.write_byte(b'-');
            }
        }
    }

    fn write_str_padded(&mut self, value: &str, width: usize) {
        self.write_str(value);
        self.pad_to_width(value.len(), width);
    }

    fn pad_to_width(&mut self, len: usize, width: usize) {
        for _ in len..width {
            self.write_byte(b' ');
        }
    }

    fn write_u64(&mut self, mut value: u64) {
        if value == 0 {
            self.write_byte(b'0');
            return;
        }

        let mut buffer = [0u8; 20];
        let mut len = 0usize;
        while value > 0 {
            buffer[len] = b'0' + (value % 10) as u8;
            value /= 10;
            len += 1;
        }

        while len > 0 {
            len -= 1;
            self.write_byte(buffer[len]);
        }
    }
}

fn decimal_len(mut value: u64) -> usize {
    if value == 0 {
        return 1;
    }

    let mut len = 0usize;
    while value > 0 {
        value /= 10;
        len += 1;
    }
    len
}

#[cfg(test)]
mod tests {
    use super::{
        ProcessCredentials, ProcessGroupError, ProcessKind, ProcessState, ProcessTable, SIG_DFL,
        SIG_IGN, SIGTERM, SIGTERM_MASK, SIGTSTP, SignalDelivery, SignalMaskHow,
        SignalTerminationError, ThreadJoinStatus, ThreadState, UserFaultKind, UserResumeContext,
        WaitTargetStatus, classify_user_fault,
    };
    use crate::user::address_space::AddressSpace;

    fn resume_context(rip: u64, rsp: u64) -> UserResumeContext {
        UserResumeContext {
            rip,
            rsp,
            rflags: 0x202,
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
            rax: 0,
        }
    }

    #[test_case]
    fn create_process_assigns_pid_and_parent() {
        let mut table = ProcessTable::new();
        let pid = table.create_process(
            Some(1),
            "test",
            "/bin/test",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0x400000),
        );

        assert_eq!(pid, 1);
        assert_eq!(table.stats().process_count, 1);
        assert_eq!(table.stats().thread_count, 1);
        assert_eq!(table.stats().kernel_task_count, 0);
        assert_eq!(table.stats().user_process_count, 1);
        assert_eq!(table.stats().ready_user_count, 0);
        assert_eq!(table.stats().running_user_count, 0);
        assert_eq!(table.stats().blocked_user_count, 0);
        assert_eq!(table.stats().foreground_user_count, 1);
        assert_eq!(table.stats().background_user_count, 0);
        assert_eq!(table.stats().ready_count, 0);
        assert_eq!(
            table.processes.get(&pid).unwrap().kind,
            ProcessKind::UserProcess
        );
        assert_eq!(table.processes.get(&pid).unwrap().main_tid(), pid);
        assert_eq!(
            table.processes.get(&pid).unwrap().main_thread().state,
            crate::user::thread::ThreadState::Created
        );
        table.mark_ready(pid);
        assert_eq!(
            table.processes.get(&pid).unwrap().state,
            ProcessState::Ready
        );
        assert_eq!(
            table.processes.get(&pid).unwrap().main_thread().state,
            crate::user::thread::ThreadState::Ready
        );
        assert_eq!(table.stats().ready_count, 1);
        assert_eq!(table.stats().ready_user_count, 1);
        assert_eq!(table.stats().ready_queue_count, 1);
        table.mark_exited(pid, 7);
        assert_eq!(
            table.processes.get(&pid).unwrap().state,
            ProcessState::Zombie
        );
        assert_eq!(
            table.processes.get(&pid).unwrap().main_thread().state,
            crate::user::thread::ThreadState::Zombie
        );
        assert_eq!(table.processes.get(&pid).unwrap().exit_status, Some(7));
        assert_eq!(table.stats().exited_count, 1);
        assert_eq!(table.stats().ready_queue_count, 0);
    }

    #[test_case]
    fn process_thread_store_uses_independent_tid_allocator() {
        let mut table = ProcessTable::new();
        let skipped_pid = table.reserve_pid();
        let pid = table.reserve_pid();
        assert_eq!(skipped_pid, 1);
        assert_eq!(pid, 2);

        table.create_reserved_process(
            pid,
            Some(1),
            "worker",
            "/bin/worker",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0x400000),
        );
        let main_tid = table.processes.get(&pid).unwrap().main_tid();
        let worker_tid = table
            .create_thread(pid)
            .expect("existing process should accept another thread");

        assert_eq!(main_tid, 1);
        assert_ne!(main_tid, pid);
        assert_eq!(worker_tid, 2);
        assert_eq!(table.thread(worker_tid).unwrap().process_id, pid);
        assert_eq!(table.processes.get(&pid).unwrap().threads.len(), 2);
        assert_eq!(table.stats().thread_count, 2);
    }

    #[test_case]
    fn secondary_user_thread_has_independent_stack_context_and_schedule_slot() {
        let mut table = ProcessTable::new();
        let pid = table.create_process(
            Some(1),
            "thread-test",
            "/bin/thread-test",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        let tid = table
            .create_user_thread(pid, 0x401000, 0x1020000, 0x1021000, 0x1020ff8, 77)
            .expect("valid secondary thread should be created");

        assert_eq!(table.stats().thread_count, 2);
        let thread = table.thread(tid).unwrap();
        assert_eq!(thread.state, crate::user::thread::ThreadState::Ready);
        assert_eq!(thread.user_stack_start, Some(0x1020000));
        assert_eq!(thread.user_stack_top, Some(0x1021000));
        assert_eq!(thread.resume_context.unwrap().rdi, 77);

        let scheduled = table
            .schedule_next_ready_user()
            .expect("secondary thread should be schedulable");
        assert_eq!(scheduled.pid, pid);
        assert_eq!(scheduled.tid, tid);
        assert_eq!(scheduled.resume_context.rip, 0x401000);
        assert_eq!(scheduled.resume_context.rsp, 0x1020ff8);
    }

    #[test_case]
    fn thread_join_blocks_caller_and_wakes_it_when_target_exits() {
        let mut table = ProcessTable::new();
        let pid = table.create_process(
            Some(1),
            "thread-join",
            "/bin/thread-join",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        table.mark_running(pid);
        let main_tid = table.processes.get(&pid).unwrap().main_tid();
        let worker_tid = table
            .create_user_thread(pid, 0x401000, 0x1020000, 0x1021000, 0x1020ff8, 42)
            .unwrap();
        let join_context = resume_context(0x400123, 0x13f0f00);

        assert_eq!(
            table.join_thread(main_tid, worker_tid, join_context),
            ThreadJoinStatus::Blocked
        );
        assert_eq!(
            table.thread(main_tid).unwrap().state,
            crate::user::thread::ThreadState::Blocked
        );
        assert_eq!(
            table.thread(main_tid).unwrap().waiting_for,
            Some(worker_tid)
        );

        let worker = table.schedule_next_ready_user().unwrap();
        assert_eq!(worker.tid, worker_tid);
        assert_eq!(table.mark_thread_exited(worker_tid), Some(pid));
        assert_eq!(
            table.thread(worker_tid).unwrap().state,
            crate::user::thread::ThreadState::Reaped
        );
        assert_eq!(
            table.thread(main_tid).unwrap().state,
            crate::user::thread::ThreadState::Ready
        );

        let resumed = table.schedule_next_ready_user().unwrap();
        assert_eq!(resumed.tid, main_tid);
        assert_eq!(resumed.resume_context, join_context);
    }

    #[test_case]
    fn generic_wait_queue_block_and_wake_preserve_syscall_context() {
        let mut table = ProcessTable::new();
        let pid = table.create_process(
            Some(1),
            "event-wait",
            "/bin/eventdemo",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        table.mark_running(pid);
        let tid = table.processes.get(&pid).unwrap().main_tid();
        let context = resume_context(0x401234, 0x13f0f00);

        assert!(table.block_thread(tid, context));
        assert_eq!(
            table.thread(tid).unwrap().state,
            crate::user::thread::ThreadState::Blocked
        );
        assert!(table.wake_thread(tid, 77));

        let resumed = table.schedule_next_ready_user().unwrap();
        assert_eq!(resumed.tid, tid);
        assert_eq!(resumed.resume_context.rax, 77);
        assert_eq!(resumed.resume_context.rip, context.rip);
    }

    #[test_case]
    fn kernel_placeholder_is_tracked_separately_from_user_processes() {
        let mut table = ProcessTable::new();
        let kernel = table.create_process(
            None,
            "kernel",
            "kernel",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0),
        );
        let user = table.create_process(
            Some(kernel),
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );

        assert_eq!(
            table.processes.get(&kernel).unwrap().kind,
            ProcessKind::KernelTask
        );
        assert_eq!(
            table.processes.get(&user).unwrap().kind,
            ProcessKind::UserProcess
        );
        assert_eq!(table.stats().kernel_task_count, 1);
        assert_eq!(table.stats().user_process_count, 1);
    }

    #[test_case]
    fn kernel_scheduler_threads_share_kernel_process_and_global_tid_namespace() {
        let mut table = ProcessTable::new();
        let kernel = table.create_process(
            None,
            "kernel",
            "kernel",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0),
        );
        let idle_tid = table.create_kernel_thread().unwrap();
        let worker_tid = table.create_kernel_thread().unwrap();

        assert_eq!(kernel, 1);
        assert_eq!(idle_tid, 2);
        assert_eq!(worker_tid, 3);
        assert_eq!(table.thread(idle_tid).unwrap().process_id, kernel);
        assert_eq!(
            table.thread(worker_tid).unwrap().kind,
            crate::user::thread::ThreadKind::Kernel
        );
        assert_eq!(table.stats().process_count, 1);
        assert_eq!(table.stats().thread_count, 3);
    }

    #[test_case]
    fn active_user_process_owns_keyboard_session() {
        let mut table = ProcessTable::new();
        let kernel = table.create_process(
            None,
            "kernel",
            "kernel",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0),
        );
        table.mark_ready(kernel);
        assert!(!table.has_active_user_session());

        let user = table.create_process(
            Some(kernel),
            "login",
            "/bin/login",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0x400000),
        );
        table.mark_ready(user);
        assert!(table.has_active_user_session());

        table.mark_exited(user, 0);
        assert!(!table.has_active_user_session());
    }

    #[test_case]
    fn user_process_state_stats_track_scheduler_lifecycle() {
        let mut table = ProcessTable::new();
        let parent_layout = crate::user::ring3::memory_layout_for_pid(1);
        let parent = table.create_process(
            Some(1),
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, parent_layout),
        );
        let child_layout = crate::user::ring3::memory_layout_for_pid(2);
        let child = table.create_process(
            Some(parent),
            "sleep",
            "/bin/sleep",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x401000, child_layout),
        );

        table.mark_ready(parent);
        table.mark_running(child);
        assert_eq!(table.stats().ready_user_count, 1);
        assert_eq!(table.stats().running_user_count, 1);
        assert_eq!(table.stats().blocked_user_count, 0);

        assert!(table.mark_waiting(parent, child, resume_context(0x401000, 0x7ff000)));
        assert_eq!(table.stats().ready_user_count, 0);
        assert_eq!(table.stats().running_user_count, 1);
        assert_eq!(table.stats().blocked_user_count, 1);
    }

    #[test_case]
    fn user_process_job_mode_stats_track_foreground_and_background() {
        let mut table = ProcessTable::new();
        let shell = table.create_process(
            Some(1),
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(1),
            ),
        );
        let background = table.create_process_with_job_mode(
            Some(shell),
            "uptime",
            "/bin/uptime",
            crate::user::program::UserProgramArg::empty(),
            crate::user::program::JobMode::Background,
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );

        assert_eq!(
            table.processes.get(&shell).unwrap().job_mode,
            crate::user::program::JobMode::Foreground
        );
        assert_eq!(
            table.processes.get(&background).unwrap().job_mode,
            crate::user::program::JobMode::Background
        );
        assert_eq!(table.stats().foreground_user_count, 1);
        assert_eq!(table.stats().background_user_count, 1);
    }

    #[test_case]
    fn ready_queue_dequeues_user_processes_in_fifo_order() {
        let mut table = ProcessTable::new();
        let first = table.create_process(
            Some(1),
            "first",
            "/bin/first",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0x400000),
        );
        let second = table.create_process(
            Some(1),
            "second",
            "/bin/second",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0x401000),
        );

        table.mark_ready(first);
        table.mark_ready(second);
        table.mark_ready(first);

        assert_eq!(table.stats().ready_count, 2);
        assert_eq!(table.stats().ready_queue_count, 2);
        assert_eq!(table.ready_queue_position(first), Some(1));
        assert_eq!(table.ready_queue_position(second), Some(2));
        assert_eq!(table.dequeue_ready(), Some(first));
        assert_eq!(
            table.processes.get(&first).unwrap().state,
            ProcessState::Running
        );
        assert_eq!(table.ready_queue_position(first), None);
        assert_eq!(table.ready_queue_position(second), Some(1));
        assert_eq!(
            table
                .processes
                .get(&first)
                .unwrap()
                .main_thread()
                .context_switches,
            1
        );
        assert_eq!(table.dequeue_ready(), Some(second));
        assert_eq!(table.ready_queue_position(second), None);
        assert_eq!(
            table
                .processes
                .get(&second)
                .unwrap()
                .main_thread()
                .context_switches,
            1
        );
        assert_eq!(table.dequeue_ready(), None);
        assert_eq!(table.stats().user_context_switches, 2);
        assert_eq!(table.stats().ready_queue_skips, 0);
    }

    #[test_case]
    fn scheduler_counts_stale_ready_queue_entries() {
        let mut table = ProcessTable::new();
        let stale = table.create_process(
            Some(1),
            "stale",
            "/bin/stale",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0x400000),
        );
        let ready = table.create_process(
            Some(1),
            "ready",
            "/bin/ready",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0x401000),
        );

        table.mark_ready(stale);
        table.mark_ready(ready);
        table.processes.get_mut(&stale).unwrap().state = ProcessState::Exited;

        assert_eq!(table.dequeue_ready(), Some(ready));
        assert_eq!(table.stats().ready_queue_skips, 1);
    }

    #[test_case]
    fn yielded_process_can_be_scheduled_for_resume() {
        let mut table = ProcessTable::new();
        let layout = crate::user::ring3::memory_layout_for_pid(2);
        let pid = table.create_process(
            Some(1),
            "yielddemo",
            "/bin/yielddemo",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, layout),
        );
        let resume_context = resume_context(0x401234, 0x7ff000);

        table.mark_running(pid);
        assert_eq!(
            table
                .processes
                .get(&pid)
                .unwrap()
                .main_thread()
                .context_switches,
            1
        );
        assert!(table.mark_yielded(pid, resume_context));
        assert_eq!(table.stats().ready_count, 1);
        assert_eq!(table.stats().ready_queue_count, 1);
        assert_eq!(
            table
                .processes
                .get(&pid)
                .unwrap()
                .main_thread()
                .last_started_tick,
            None
        );

        let resume = table
            .schedule_next_ready_user()
            .expect("yielded process should be resumable");
        assert_eq!(resume.pid, pid);
        assert_eq!(resume.name, "yielddemo");
        assert_eq!(resume.resume_context, resume_context);
        assert_eq!(
            table.processes.get(&pid).unwrap().state,
            ProcessState::Running
        );
        assert_eq!(
            table
                .processes
                .get(&pid)
                .unwrap()
                .main_thread()
                .context_switches,
            2
        );
        assert!(
            table
                .processes
                .get(&pid)
                .unwrap()
                .main_thread()
                .last_started_tick
                .is_some()
        );
    }

    #[test_case]
    fn scheduler_switches_between_two_yielded_user_processes_in_fifo_order() {
        let mut table = ProcessTable::new();
        let first = table.create_process(
            Some(1),
            "yield-a",
            "/bin/yielddemo",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        let second = table.create_process(
            Some(1),
            "yield-b",
            "/bin/yielddemo",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(3),
            ),
        );
        let first_context = resume_context(0x401111, 0x7ff100);
        let second_context = resume_context(0x402222, 0x7fe200);

        table.mark_running(first);
        assert!(table.mark_yielded(first, first_context));
        table.mark_running(second);
        assert!(table.mark_yielded(second, second_context));

        assert_eq!(table.stats().ready_user_count, 2);
        assert_eq!(table.ready_queue_position(first), Some(1));
        assert_eq!(table.ready_queue_position(second), Some(2));

        let first_resume = table
            .schedule_next_ready_user()
            .expect("first yielded process should resume first");
        assert_eq!(first_resume.pid, first);
        assert_eq!(first_resume.resume_context, first_context);
        assert_eq!(table.ready_queue_position(second), Some(1));
        table.mark_exited(first, 0);

        let second_resume = table
            .schedule_next_ready_user()
            .expect("second yielded process should resume second");
        assert_eq!(second_resume.pid, second);
        assert_eq!(second_resume.resume_context, second_context);
        assert_eq!(table.stats().ready_queue_count, 0);
        assert_eq!(table.stats().ready_queue_skips, 0);
        assert_eq!(table.stats().user_context_switches, 4);
    }

    #[test_case]
    fn timer_preemption_preserves_full_register_context() {
        let mut table = ProcessTable::new();
        let running = table.create_process(
            Some(1),
            "cpu-bound",
            "/bin/cpu-bound",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        let ready = table.create_process(
            Some(1),
            "shell",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(3),
            ),
        );
        let running_context = UserResumeContext {
            rax: 0xaabb_ccdd_eeff_0011,
            ..resume_context(0x401111, 0x7ff100)
        };
        let ready_context = resume_context(0x402222, 0x7fe200);

        table.mark_running(running);
        table.mark_running(ready);
        assert!(table.mark_yielded(ready, ready_context));

        let next = table
            .preempt_running_user(running, running_context)
            .expect("ready process should preempt running process");
        assert_eq!(next.pid, ready);
        assert_eq!(next.resume_context, ready_context);

        table.mark_exited(ready, 0);
        let resumed = table
            .schedule_next_ready_user()
            .expect("preempted process should remain runnable");
        assert_eq!(resumed.pid, running);
        assert_eq!(resumed.resume_context, running_context);
        assert_eq!(resumed.resume_context.rax, 0xaabb_ccdd_eeff_0011);
    }

    #[test_case]
    fn scheduler_counts_ready_user_without_resume_context() {
        let mut table = ProcessTable::new();
        let stale = table.create_process(
            Some(1),
            "stale",
            "/bin/stale",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );

        table.mark_ready(stale);
        assert_eq!(table.schedule_next_ready_user(), None);
        assert_eq!(table.stats().ready_queue_skips, 1);
    }

    #[test_case]
    fn ready_except_ignores_current_process() {
        let mut table = ProcessTable::new();
        let current = table.create_process(
            Some(1),
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0x400000),
        );
        let other = table.create_process(
            Some(1),
            "yielddemo",
            "/bin/yielddemo",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0x401000),
        );

        table.mark_ready(current);
        assert!(!table.has_ready_process_except(current));
        table.mark_ready(other);
        assert!(table.has_ready_process_except(current));
    }

    #[test_case]
    fn parent_can_reap_zombie_child() {
        let mut table = ProcessTable::new();
        let parent_layout = crate::user::ring3::memory_layout_for_pid(1);
        let parent = table.create_process(
            None,
            "parent",
            "/bin/parent",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, parent_layout),
        );
        let child_layout = crate::user::ring3::memory_layout_for_pid(2);
        let child = table.create_process(
            Some(parent),
            "child",
            "/bin/child",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, child_layout),
        );

        table.mark_exited(child, 3);
        assert!(table.mark_waiting(parent, child, resume_context(0x401000, 0x7ff000),));
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Zombie
        );

        assert_eq!(table.reap_child(parent, child), Some((child, "child", 3)));
        assert_eq!(table.processes.get(&parent).unwrap().waiting_for, None);
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Reaped
        );
    }

    #[test_case]
    fn parent_waits_running_child_until_child_exits() {
        let mut table = ProcessTable::new();
        let parent_layout = crate::user::ring3::memory_layout_for_pid(1);
        let parent = table.create_process(
            None,
            "parent",
            "/bin/parent",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, parent_layout),
        );
        let child_layout = crate::user::ring3::memory_layout_for_pid(2);
        let child = table.create_process(
            Some(parent),
            "child",
            "/bin/child",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x401000, child_layout),
        );
        let resume_context = resume_context(0x401123, 0x7ffabc);

        table.mark_running(child);
        assert!(table.mark_waiting(parent, child, resume_context));
        assert_eq!(
            table.processes.get(&parent).unwrap().state,
            ProcessState::Blocked
        );
        assert_eq!(
            table.wait_target_status(parent, child),
            WaitTargetStatus::NotExited {
                state: ProcessState::Running
            }
        );
        assert_eq!(table.resume_waiting_parent(parent, child), None);

        table.mark_exited(child, 4);
        let resume = table
            .resume_waiting_parent(parent, child)
            .expect("exited child should resume waiting parent");

        assert_eq!(resume.parent_pid, parent);
        assert_eq!(resume.child_pid, child);
        assert_eq!(resume.status, 4);
        assert_eq!(resume.resume_context, resume_context);
        assert_eq!(
            table.processes.get(&parent).unwrap().state,
            ProcessState::Running
        );
        assert_eq!(table.processes.get(&parent).unwrap().waiting_for, None);
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Reaped
        );
    }

    #[test_case]
    fn parent_can_reap_zombie_child_without_blocking_first() {
        let mut table = ProcessTable::new();
        let parent_layout = crate::user::ring3::memory_layout_for_pid(1);
        let parent = table.create_process(
            None,
            "parent",
            "/bin/parent",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, parent_layout),
        );
        let child_layout = crate::user::ring3::memory_layout_for_pid(2);
        let child = table.create_process(
            Some(parent),
            "child",
            "/bin/child",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, child_layout),
        );

        table.mark_exited(child, 9);

        assert_eq!(table.reap_child(parent, child), Some((child, "child", 9)));
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Reaped
        );
    }

    #[test_case]
    fn sigterm_terminates_ready_user_process_with_signal_status() {
        let mut table = ProcessTable::new();
        let caller = table.create_process(
            None,
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(1),
            ),
        );
        let target = table.create_process_with_job_mode(
            Some(caller),
            "sleep",
            "/bin/sleep",
            crate::user::program::UserProgramArg::empty(),
            crate::user::program::JobMode::Background,
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        table.mark_ready(target);

        let report = table
            .terminate_user_process(caller, target, 143)
            .expect("root caller should terminate background child");

        assert_eq!(report.pid, target);
        assert_eq!(report.status, 143);
        assert_eq!(report.job_mode, crate::user::program::JobMode::Background);
        assert!(!report.parent_woken);
        assert_eq!(
            table.processes.get(&target).unwrap().state,
            ProcessState::Zombie
        );
        assert_eq!(table.processes.get(&target).unwrap().exit_status, Some(143));
        assert_eq!(table.ready_queue_position(target), None);
    }

    #[test_case]
    fn sigterm_wakes_parent_blocked_in_waitpid() {
        let mut table = ProcessTable::new();
        let parent = table.create_process(
            None,
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(1),
            ),
        );
        let child = table.create_process(
            Some(parent),
            "sleep",
            "/bin/sleep",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        let killer = table.create_process(
            None,
            "killer",
            "/bin/kill",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x402000,
                crate::user::ring3::memory_layout_for_pid(3),
            ),
        );
        let waiting_context = resume_context(0x400123, 0x7ff000);
        table.mark_running(parent);
        assert!(table.mark_waiting(parent, child, waiting_context));

        let report = table
            .terminate_user_process(killer, child, 143)
            .expect("root caller should terminate waited child");

        assert!(report.parent_woken);
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Reaped
        );
        let parent = table.processes.get(&parent).unwrap();
        assert_eq!(parent.state, ProcessState::Ready);
        assert_eq!(parent.waiting_for, None);
        assert_eq!(parent.main_thread().resume_context.unwrap().rax, 143);
    }

    #[test_case]
    fn sigterm_enforces_process_kind_and_uid_permissions() {
        let mut table = ProcessTable::new();
        let kernel = table.create_process(
            None,
            "kernel",
            "kernel",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0),
        );
        let root_target = table.create_process(
            None,
            "root-task",
            "/bin/root-task",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        let user = table.create_process(
            None,
            "user-task",
            "/bin/user-task",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(3),
            ),
        );
        assert!(table.set_credentials(user, ProcessCredentials::from_username("hafifi")));

        assert_eq!(
            table.terminate_user_process(user, kernel, 143),
            Err(SignalTerminationError::KernelTask)
        );
        assert_eq!(
            table.terminate_user_process(user, root_target, 143),
            Err(SignalTerminationError::PermissionDenied)
        );
    }

    #[test_case]
    fn blocked_sigterm_becomes_pending_until_unblocked() {
        let mut table = ProcessTable::new();
        let caller = table.create_process(
            None,
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(1),
            ),
        );
        let target = table.create_process(
            Some(caller),
            "signaldemo",
            "/bin/signaldemo",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        let blocked = table
            .update_signal_mask(target, SignalMaskHow::Block, SIGTERM_MASK)
            .expect("target signal mask should update");
        assert_eq!(blocked.old_mask, 0);
        assert_eq!(blocked.new_mask, SIGTERM_MASK);

        assert_eq!(
            table.send_signal(caller, target, SIGTERM),
            Ok(SignalDelivery::Pending {
                pid: target,
                name: "signaldemo",
                signal: SIGTERM,
                pending_signals: SIGTERM_MASK,
            })
        );
        assert_eq!(table.pending_signals(target), Some(SIGTERM_MASK));
        assert_ne!(
            table.processes.get(&target).unwrap().state,
            ProcessState::Zombie
        );

        let unblocked = table
            .update_signal_mask(target, SignalMaskHow::Unblock, SIGTERM_MASK)
            .expect("target signal mask should update");
        assert_eq!(unblocked.old_mask, SIGTERM_MASK);
        assert_eq!(unblocked.new_mask, 0);
        assert_eq!(unblocked.delivered_signal, Some(SIGTERM));
        assert_eq!(table.pending_signals(target), Some(0));
    }

    #[test_case]
    fn signal_setmask_reports_previous_mask() {
        let mut table = ProcessTable::new();
        let target = table.create_process(
            None,
            "signaldemo",
            "/bin/signaldemo",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(1),
            ),
        );

        let first = table
            .update_signal_mask(target, SignalMaskHow::SetMask, SIGTERM_MASK)
            .unwrap();
        let second = table
            .update_signal_mask(target, SignalMaskHow::SetMask, 0)
            .unwrap();

        assert_eq!(first.old_mask, 0);
        assert_eq!(first.new_mask, SIGTERM_MASK);
        assert_eq!(second.old_mask, SIGTERM_MASK);
        assert_eq!(second.new_mask, 0);
    }

    #[test_case]
    fn sigterm_handler_replaces_and_restores_saved_context() {
        let mut table = ProcessTable::new();
        let caller = table.create_process(
            None,
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(1),
            ),
        );
        let target = table.create_process(
            Some(caller),
            "handlerdemo",
            "/bin/handlerdemo",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        let original = resume_context(0x401234, 0x13f0f00);
        let handler = 0x402000;
        assert_eq!(
            table.set_signal_disposition(target, SIGTERM, handler),
            Some(SIG_DFL)
        );
        table.mark_running(target);
        assert!(table.mark_sleeping(target, 999, original));

        assert_eq!(
            table.send_signal(caller, target, SIGTERM),
            Ok(SignalDelivery::Handled {
                pid: target,
                name: "handlerdemo",
                signal: SIGTERM,
                handler,
            })
        );

        let thread = table.processes.get(&target).unwrap().main_thread();
        assert_eq!(thread.state, ThreadState::Ready);
        assert_eq!(thread.signal_return_context, Some(original));
        assert_eq!(thread.resume_context.unwrap().rip, handler);
        assert_eq!(thread.resume_context.unwrap().rdi, SIGTERM);

        let scheduled = table.schedule_next_ready_user().unwrap();
        assert_eq!(scheduled.pid, target);
        assert_eq!(scheduled.resume_context.rip, handler);
        assert!(table.restore_signal_context(target, scheduled.tid));
        let restored = table.schedule_next_ready_user().unwrap();
        assert_eq!(restored.resume_context, original);
    }

    #[test_case]
    fn ignored_sigterm_does_not_terminate_target() {
        let mut table = ProcessTable::new();
        let caller = table.create_process(
            None,
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(1),
            ),
        );
        let target = table.create_process(
            Some(caller),
            "worker",
            "/bin/worker",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        assert_eq!(
            table.set_signal_disposition(target, SIGTERM, SIG_IGN),
            Some(SIG_DFL)
        );
        assert_eq!(
            table.send_signal(caller, target, SIGTERM),
            Ok(SignalDelivery::Ignored {
                pid: target,
                name: "worker",
                signal: SIGTERM,
            })
        );
        assert_eq!(table.processes.get(&target).unwrap().exit_status, None);
    }

    #[test_case]
    fn child_inherits_parent_process_group_and_session() {
        let mut table = ProcessTable::new();
        let kernel = table.create_process(
            None,
            "kernel",
            "kernel",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0),
        );
        let init = table.create_process(
            Some(kernel),
            "init",
            "/bin/init",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        let shell = table.create_process(
            Some(init),
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(3),
            ),
        );

        assert_eq!(table.process_group_id(init), Some(init));
        assert_eq!(table.session_id(init), Some(init));
        assert_eq!(table.process_group_id(shell), Some(init));
        assert_eq!(table.session_id(shell), Some(init));
    }

    #[test_case]
    fn process_can_create_group_then_child_can_join_it() {
        let mut table = ProcessTable::new();
        let parent = table.create_process(
            None,
            "parent",
            "/bin/parent",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(1),
            ),
        );
        let first = table.create_process(
            Some(parent),
            "first",
            "/bin/first",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        let second = table.create_process(
            Some(parent),
            "second",
            "/bin/second",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x402000,
                crate::user::ring3::memory_layout_for_pid(3),
            ),
        );

        assert_eq!(table.set_process_group(parent, first, 0), Ok(first));
        assert_eq!(table.set_process_group(parent, second, first), Ok(first));
        assert_eq!(table.process_group_id(first), Some(first));
        assert_eq!(table.process_group_id(second), Some(first));
        assert_eq!(table.session_id(first), table.session_id(parent));
    }

    #[test_case]
    fn setsid_creates_new_session_and_rejects_group_leader() {
        let mut table = ProcessTable::new();
        let parent = table.create_process(
            None,
            "parent",
            "/bin/parent",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(1),
            ),
        );
        let child = table.create_process(
            Some(parent),
            "child",
            "/bin/child",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );

        assert_eq!(table.create_session(child), Ok(child));
        assert_eq!(table.session_id(child), Some(child));
        assert_eq!(table.process_group_id(child), Some(child));
        assert_eq!(
            table.create_session(child),
            Err(ProcessGroupError::GroupLeader)
        );
        assert_eq!(
            table.set_process_group(child, child, 0),
            Err(ProcessGroupError::SessionLeader)
        );
    }

    #[test_case]
    fn sigtstp_stops_child_wakes_parent_and_sigcont_resumes() {
        let mut table = ProcessTable::new();
        let parent = table.create_process(
            None,
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(1),
            ),
        );
        let child = table.create_process(
            Some(parent),
            "sleep",
            "/bin/sleep",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x401000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        table.mark_running(child);
        assert!(table.mark_sleeping(child, 500, resume_context(0x401234, 0x13f0f00)));
        table.mark_running(parent);
        assert!(table.mark_waiting(parent, child, resume_context(0x400456, 0x13f2f00)));

        let report = table
            .stop_user_process(child, SIGTSTP)
            .expect("foreground child should stop");
        assert!(report.stopped);
        assert!(report.parent_woken);
        assert_eq!(report.status, 148);
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Stopped
        );
        assert_eq!(
            table.processes.get(&parent).unwrap().state,
            ProcessState::Ready
        );
        assert_eq!(
            table
                .processes
                .get(&parent)
                .unwrap()
                .main_thread()
                .resume_context
                .unwrap()
                .rax,
            148
        );

        assert!(table.continue_user_process(parent, child));
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Ready
        );
    }

    #[test_case]
    fn background_reaper_skips_children_being_waited_on() {
        let mut table = ProcessTable::new();
        let parent_layout = crate::user::ring3::memory_layout_for_pid(1);
        let parent = table.create_process(
            None,
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, parent_layout),
        );
        let foreground = table.create_process(
            Some(parent),
            "cat",
            "/bin/cat",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(2),
            ),
        );
        let background = table.create_process_with_job_mode(
            Some(parent),
            "uptime",
            "/bin/uptime",
            crate::user::program::UserProgramArg::empty(),
            crate::user::program::JobMode::Background,
            AddressSpace::kernel_shared_user(
                0x400000,
                crate::user::ring3::memory_layout_for_pid(3),
            ),
        );

        table.mark_exited(foreground, 0);
        table.mark_exited(background, 0);
        assert!(table.mark_waiting(parent, foreground, resume_context(0x401000, 0x7ff000),));

        assert_eq!(table.reap_unwaited_zombies(), 1);
        assert_eq!(
            table.processes.get(&foreground).unwrap().state,
            ProcessState::Zombie
        );
        assert_eq!(
            table.processes.get(&background).unwrap().state,
            ProcessState::Reaped
        );
    }

    #[test_case]
    fn wait_target_status_explains_invalid_targets() {
        let mut table = ProcessTable::new();
        let parent_layout = crate::user::ring3::memory_layout_for_pid(1);
        let parent = table.create_process(
            None,
            "parent",
            "/bin/parent",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, parent_layout),
        );
        let other_layout = crate::user::ring3::memory_layout_for_pid(2);
        let other_parent = table.create_process(
            None,
            "other",
            "/bin/other",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, other_layout),
        );
        let child_layout = crate::user::ring3::memory_layout_for_pid(3);
        let child = table.create_process(
            Some(parent),
            "child",
            "/bin/child",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, child_layout),
        );

        assert_eq!(
            table.wait_target_status(parent, 999),
            WaitTargetStatus::Missing
        );
        assert_eq!(
            table.wait_target_status(other_parent, child),
            WaitTargetStatus::NotChild {
                actual_parent: Some(parent)
            }
        );
        assert_eq!(
            table.wait_target_status(parent, child),
            WaitTargetStatus::NotExited {
                state: ProcessState::Created
            }
        );

        table.mark_exited(child, 0);
        assert_eq!(
            table.wait_target_status(parent, child),
            WaitTargetStatus::Reapable
        );
    }

    #[test_case]
    fn classifies_user_fault_addresses() {
        let layout = crate::user::ring3::memory_layout_for_pid_with_program_size(4, 0x2000);

        assert_eq!(classify_user_fault(0, layout), UserFaultKind::Null);
        assert_eq!(
            classify_user_fault(0xffff_8000_0000_0000, layout),
            UserFaultKind::KernelSpace
        );
        assert_eq!(
            classify_user_fault(layout.stack_start - 8, layout),
            UserFaultKind::StackGuard
        );
        assert_eq!(
            classify_user_fault(layout.program_start, layout),
            UserFaultKind::UserRange
        );
        assert_eq!(
            classify_user_fault(layout.stack_top - 8, layout),
            UserFaultKind::UserRange
        );
        assert_eq!(
            classify_user_fault(layout.program_end + 0x1000, layout),
            UserFaultKind::OutsideUserRange
        );
    }

    #[test_case]
    fn init_reaper_collects_zombies_whose_parent_is_gone() {
        let mut table = ProcessTable::new();
        let parent_layout = crate::user::ring3::memory_layout_for_pid(2);
        let parent = table.create_process(
            Some(1),
            "parent",
            "/bin/parent",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, parent_layout),
        );
        let child_layout = crate::user::ring3::memory_layout_for_pid(3);
        let child = table.create_process(
            Some(parent),
            "child",
            "/bin/child",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, child_layout),
        );

        table.mark_exited(parent, 0);
        table.mark_exited(child, 0);

        assert_eq!(table.reap_orphan_zombies(), 2);
        assert_eq!(
            table.processes.get(&parent).unwrap().state,
            ProcessState::Reaped
        );
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Reaped
        );
    }

    #[test_case]
    fn init_reaper_collects_zombie_child_of_reaped_parent() {
        let mut table = ProcessTable::new();
        let parent_layout = crate::user::ring3::memory_layout_for_pid(2);
        let parent = table.create_process(
            Some(1),
            "init",
            "/bin/init",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, parent_layout),
        );
        let child_layout = crate::user::ring3::memory_layout_for_pid(3);
        let child = table.create_process(
            Some(parent),
            "login",
            "/bin/login",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, child_layout),
        );

        table.mark_exited(parent, 0);
        table.mark_reaped(parent);
        table.mark_exited(child, 0);

        assert_eq!(table.reap_orphan_zombies(), 1);
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Reaped
        );
    }

    #[test_case]
    fn init_reaper_leaves_children_for_live_pid_one() {
        let mut table = ProcessTable::new();
        let init_layout = crate::user::ring3::memory_layout_for_pid(1);
        let init = table.create_process(
            None,
            "init",
            "/bin/init",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, init_layout),
        );
        assert_eq!(init, 1);

        let layout = crate::user::ring3::memory_layout_for_pid(2);
        let child = table.create_process(
            Some(init),
            "login",
            "/bin/login",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, layout),
        );

        table.mark_exited(child, 0);

        assert_eq!(table.reap_orphan_zombies(), 0);
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Zombie
        );
    }

    #[test_case]
    fn compact_reaped_history_removes_oldest_reaped_processes() {
        let mut table = ProcessTable::new();
        let kernel = table.create_process(
            None,
            "kernel",
            "kernel",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0),
        );
        table.mark_ready(kernel);

        for index in 0..5 {
            let layout = crate::user::ring3::memory_layout_for_pid(index + 2);
            let pid = table.create_process(
                Some(kernel),
                "child",
                "/bin/child",
                crate::user::program::UserProgramArg::empty(),
                AddressSpace::kernel_shared_user(0x400000, layout),
            );
            table.mark_exited(pid, 0);
            table.mark_reaped(pid);
        }

        assert_eq!(table.compact_reaped_history(2), 3);
        assert!(table.processes.contains_key(&kernel));
        assert_eq!(table.stats().process_count, 3);
        assert_eq!(
            table
                .processes
                .values()
                .filter(|process| process.state == ProcessState::Reaped)
                .count(),
            2
        );
    }

    #[test_case]
    fn reserved_process_uses_given_pid_and_advances_next_pid() {
        let mut table = ProcessTable::new();
        let pid = table.reserve_pid();
        assert_eq!(pid, 1);

        let layout = crate::user::ring3::memory_layout_for_pid(pid);
        table.create_reserved_process(
            pid,
            Some(1),
            "child",
            "/bin/child",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, layout),
        );

        assert_eq!(table.stats().process_count, 1);
        assert_eq!(table.stats().next_pid, 2);
        assert_eq!(table.processes.get(&pid).unwrap().parent_pid, Some(1));
    }

    #[test_case]
    fn user_task_address_space_records_entry_and_stack() {
        let layout = crate::user::ring3::memory_layout_for_pid(2);
        let address_space =
            AddressSpace::kernel_shared_user(crate::user::ring3::FIRST_USER_ENTRY, layout);

        assert_eq!(
            address_space.entry_point,
            crate::user::ring3::FIRST_USER_ENTRY
        );
        assert_eq!(
            address_space.user_stack_top(),
            crate::user::ring3::FIRST_USER_STACK_TOP
        );
        assert_eq!(
            address_space.layout.stack_start,
            crate::user::ring3::FIRST_USER_STACK_TOP - crate::user::ring3::USER_STACK_SIZE
        );
    }

    #[test_case]
    fn elf_style_program_footprint_tracks_highest_mapped_byte() {
        let program_size = 0x10a0 + 252;
        let layout = crate::user::ring3::memory_layout_for_pid_with_program_size(4, program_size);

        assert_eq!(layout.program_start, crate::user::ring3::FIRST_USER_ENTRY);
        assert_eq!(
            layout.program_end,
            crate::user::ring3::FIRST_USER_ENTRY + 0x2000
        );
    }
}
