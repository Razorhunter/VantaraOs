use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use lazy_static::lazy_static;
use spin::Mutex;

use crate::user::address_space::AddressSpace;
use crate::user::thread::{Thread, ThreadKind, ThreadState, Tid};

pub type Pid = u32;
pub const DEFAULT_REAPED_HISTORY_LIMIT: usize = 16;
pub const USERNAME_MAX_LEN: usize = 16;
const USER_PREEMPT_QUANTUM_TICKS: u64 = 10;
static CURRENT_USER_PID_ATOMIC: AtomicU32 = AtomicU32::new(0);
static TIMER_PREEMPT_CHECKS: AtomicU64 = AtomicU64::new(0);
static TIMER_PREEMPT_REQUESTS: AtomicU64 = AtomicU64::new(0);
static TIMER_PREEMPT_YIELDS: AtomicU64 = AtomicU64::new(0);
static TIMER_PREEMPT_PENDING: AtomicBool = AtomicBool::new(false);
static LAST_PREEMPT_REQUEST_TICK: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Created,
    Ready,
    Running,
    Blocked,
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
    pub name: &'static str,
    pub program_path: &'static str,
    pub kind: ProcessKind,
    pub main_thread: Thread,
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
    pub resume_context: Option<UserResumeContext>,
    pub context_switches: u64,
    pub runtime_ticks: u64,
    pub last_started_tick: Option<u64>,
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
    pub name: &'static str,
    pub waiting_for: Option<Pid>,
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
    fn set_state(&mut self, state: ProcessState) {
        self.state = state;
        self.main_thread.state = thread_state_for_process(state);
    }
}

const fn thread_state_for_process(state: ProcessState) -> ThreadState {
    match state {
        ProcessState::Created => ThreadState::Created,
        ProcessState::Ready => ThreadState::Ready,
        ProcessState::Running => ThreadState::Running,
        ProcessState::Blocked => ThreadState::Blocked,
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

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

pub struct ProcessTable {
    processes: ProcessStore,
    ready_queue: Vec<Pid>,
    next_pid: Pid,
    user_context_switches: u64,
    ready_queue_skips: u64,
}

impl ProcessTable {
    pub const fn new() -> Self {
        Self {
            processes: ProcessStore::new(),
            ready_queue: Vec::new(),
            next_pid: 1,
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
        let thread_kind = match kind {
            ProcessKind::KernelTask => ThreadKind::Kernel,
            ProcessKind::UserProcess => ThreadKind::User,
        };

        self.processes.insert(
            pid,
            Process {
                pid,
                parent_pid,
                name,
                program_path,
                kind,
                main_thread: Thread::main(pid, thread_kind),
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
                resume_context: None,
                context_switches: 0,
                runtime_ticks: 0,
                last_started_tick: None,
            },
        );
    }

    pub fn credentials_for(&self, pid: Pid) -> Option<ProcessCredentials> {
        self.processes.get(&pid).map(|process| process.credentials)
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
            self.enqueue_ready(pid);
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
        process.resume_context = None;
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
        process.resume_context = Some(resume_context);
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
        process.resume_context = Some(resume_context);
        process.set_state(ProcessState::Blocked);
        true
    }

    pub fn mark_yielded(&mut self, pid: Pid, resume_context: UserResumeContext) -> bool {
        let Some(process) = self.processes.get_mut(&pid) else {
            return false;
        };
        if process.state != ProcessState::Running {
            return false;
        }

        stop_process_run(process);
        process.resume_context = Some(resume_context);
        process.sleeping_until_tick = None;
        process.set_state(ProcessState::Ready);
        self.enqueue_ready(pid);
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
                parent.resume_context = None;
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

        let resume_context = parent.resume_context?;
        let p4_frame = parent.address_space.p4_frame;

        if let Some(child) = self.processes.get_mut(&child_pid) {
            child.set_state(ProcessState::Reaped);
        }
        if let Some(parent) = self.processes.get_mut(&parent_pid) {
            parent.waiting_for = None;
            parent.sleeping_until_tick = None;
            parent.resume_context = None;
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
            if let Some(process) = self.processes.get_mut(&pid) {
                process.sleeping_until_tick = None;
                process.set_state(ProcessState::Ready);
                self.enqueue_ready(pid);
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
            self.processes.remove(pid);
            self.remove_from_ready_queue(*pid);
        }

        remove_count
    }

    pub fn dequeue_ready(&mut self) -> Option<Pid> {
        while !self.ready_queue.is_empty() {
            let pid = self.ready_queue.remove(0);
            let Some(process) = self.processes.get_mut(&pid) else {
                self.ready_queue_skips = self.ready_queue_skips.saturating_add(1);
                continue;
            };
            if process.state != ProcessState::Ready {
                self.ready_queue_skips = self.ready_queue_skips.saturating_add(1);
                continue;
            }

            process.set_state(ProcessState::Running);
            process.context_switches = process.context_switches.saturating_add(1);
            process.last_started_tick = Some(crate::timer::ticks());
            self.user_context_switches += 1;
            return Some(pid);
        }

        None
    }

    pub fn schedule_next_ready_user(&mut self) -> Option<ScheduledUserResume> {
        while !self.ready_queue.is_empty() {
            let pid = self.ready_queue.remove(0);
            let Some(process) = self.processes.get_mut(&pid) else {
                self.ready_queue_skips = self.ready_queue_skips.saturating_add(1);
                continue;
            };
            if process.state != ProcessState::Ready {
                self.ready_queue_skips = self.ready_queue_skips.saturating_add(1);
                continue;
            }

            let Some(resume_context) = process.resume_context.take() else {
                self.ready_queue_skips = self.ready_queue_skips.saturating_add(1);
                continue;
            };
            process.set_state(ProcessState::Running);
            process.context_switches = process.context_switches.saturating_add(1);
            process.last_started_tick = Some(crate::timer::ticks());
            self.user_context_switches += 1;
            return Some(ScheduledUserResume {
                pid,
                tid: process.main_thread.tid,
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
        if !self.has_ready_process_except(pid) || !self.mark_yielded(pid, resume_context) {
            return None;
        }

        self.schedule_next_ready_user()
    }

    pub fn record_user_yield(&mut self, pid: Pid) -> bool {
        let Some(process) = self.processes.get(&pid) else {
            return false;
        };
        if process.state != ProcessState::Running {
            return false;
        }

        self.user_context_switches += 1;
        true
    }

    pub fn has_ready_process_except(&self, pid: Pid) -> bool {
        self.ready_queue.iter().any(|queued_pid| {
            *queued_pid != pid
                && self
                    .processes
                    .get(queued_pid)
                    .is_some_and(|process| process.state == ProcessState::Ready)
        })
    }

    pub fn ready_queue_position(&self, pid: Pid) -> Option<usize> {
        self.ready_queue
            .iter()
            .position(|queued_pid| *queued_pid == pid)
            .map(|index| index + 1)
    }

    pub fn contains_pid(&self, pid: Pid) -> bool {
        self.processes.contains_key(&pid)
    }

    fn should_init_reap(&self, parent_pid: Option<Pid>) -> bool {
        let Some(parent_pid) = parent_pid else {
            return false;
        };

        if parent_pid == 1 {
            return true;
        }

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
            thread_count: self.processes.len(),
        }
    }

    fn enqueue_ready(&mut self, pid: Pid) {
        if !self.ready_queue.contains(&pid) {
            self.ready_queue.push(pid);
        }
    }

    fn remove_from_ready_queue(&mut self, pid: Pid) {
        self.ready_queue.retain(|queued| *queued != pid);
    }

    fn start_process_run(&mut self, pid: Pid) {
        if let Some(process) = self.processes.get_mut(&pid) {
            TIMER_PREEMPT_PENDING.store(false, Ordering::Relaxed);
            CURRENT_USER_PID_ATOMIC.store(pid, Ordering::Relaxed);
            process.set_state(ProcessState::Running);
            process.context_switches = process.context_switches.saturating_add(1);
            process.last_started_tick = Some(crate::timer::ticks());
            self.user_context_switches += 1;
        }
    }
}

fn stop_process_run(process: &mut Process) {
    if CURRENT_USER_PID_ATOMIC.load(Ordering::Relaxed) == process.pid {
        CURRENT_USER_PID_ATOMIC.store(0, Ordering::Relaxed);
        TIMER_PREEMPT_PENDING.store(false, Ordering::Relaxed);
    }
    if let Some(started_at) = process.last_started_tick.take() {
        let elapsed = crate::timer::ticks().saturating_sub(started_at);
        process.runtime_ticks = process.runtime_ticks.saturating_add(elapsed);
    }
}

fn runtime_ticks_snapshot(process: &Process, now: u64) -> u64 {
    let active_elapsed = match (process.state, process.last_started_tick) {
        (ProcessState::Running, Some(started_at)) => now.saturating_sub(started_at),
        _ => 0,
    };
    process.runtime_ticks.saturating_add(active_elapsed)
}

lazy_static! {
    pub static ref PROCESS_TABLE: Mutex<ProcessTable> = Mutex::new(ProcessTable::new());
    static ref CURRENT_USER_PROCESS: Mutex<Option<Pid>> = Mutex::new(None);
    static ref LAST_EXITED_PROCESS: Mutex<Option<Pid>> = Mutex::new(None);
}

fn set_current_user_process(pid: Option<Pid>) {
    *CURRENT_USER_PROCESS.lock() = pid;
    CURRENT_USER_PID_ATOMIC.store(pid.unwrap_or(0), Ordering::Relaxed);
    if pid.is_none() {
        TIMER_PREEMPT_PENDING.store(false, Ordering::Relaxed);
    }
}

fn take_current_user_process() -> Option<Pid> {
    let pid = CURRENT_USER_PROCESS.lock().take();
    CURRENT_USER_PID_ATOMIC.store(0, Ordering::Relaxed);
    TIMER_PREEMPT_PENDING.store(false, Ordering::Relaxed);
    pid
}

pub fn init_process_table() {
    let mut table = PROCESS_TABLE.lock();
    if table.stats().process_count == 0 {
        let pid = table.create_process(
            None,
            "kernel",
            "kernel",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_placeholder(0),
        );
        table.mark_ready(pid);
        crate::serial_println!("[PROCESS] Created placeholder init process pid={}", pid);
    }
}

pub fn stats() -> ProcessStats {
    PROCESS_TABLE.lock().stats()
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

    let pid = CURRENT_USER_PID_ATOMIC.load(Ordering::Relaxed);
    if pid == 0 {
        return None;
    }

    let mut table = PROCESS_TABLE.lock();
    let resume = table.preempt_running_user(pid, resume_context)?;
    drop(table);

    *CURRENT_USER_PROCESS.lock() = Some(resume.pid);
    CURRENT_USER_PID_ATOMIC.store(resume.pid, Ordering::Relaxed);
    TIMER_PREEMPT_PENDING.store(false, Ordering::Relaxed);
    TIMER_PREEMPT_YIELDS.fetch_add(1, Ordering::Relaxed);
    Some(resume)
}

pub fn record_user_yield(pid: Pid) -> bool {
    PROCESS_TABLE.lock().record_user_yield(pid)
}

pub fn has_ready_process_except(pid: Pid) -> bool {
    PROCESS_TABLE.lock().has_ready_process_except(pid)
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
    set_current_user_process(Some(pid));
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
    let pid = take_current_user_process()?;
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
    let table = PROCESS_TABLE.lock();
    let process = table.processes.get(&pid)?;
    if process.state != ProcessState::Blocked {
        return None;
    }

    let report = BlockedUserReport {
        pid,
        name: process.name,
        waiting_for: process.waiting_for,
        resume_context: process.resume_context,
    };
    drop(table);
    set_current_user_process(None);
    Some(report)
}

pub fn checkout_current_user_if_yielded(
    resume_context: UserResumeContext,
) -> Option<YieldedUserReport> {
    let pid = take_current_user_process()?;
    let mut table = PROCESS_TABLE.lock();
    if !table.mark_yielded(pid, resume_context) {
        set_current_user_process(Some(pid));
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
    set_current_user_process(Some(resume.pid));
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

pub fn resume_waiting_parent(parent_pid: Pid, child_pid: Pid) -> Option<WaitResume> {
    let mut table = PROCESS_TABLE.lock();
    let resume = table.resume_waiting_parent(parent_pid, child_pid)?;
    set_current_user_process(Some(parent_pid));
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
    let pid = take_current_user_process()?;
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
    let pid = take_current_user_process()?;
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
    let pid = current_user_pid()?;
    PROCESS_TABLE
        .lock()
        .processes
        .get(&pid)
        .map(|process| process.main_thread.tid)
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
            "pid={} tid={} ppid={:?} kind={:?} mode={:?} name={} state={:?} tstate={:?} status={:?} wait={:?} ctx={} ticks={} resume={:?} as={:?} p4={:?} p4ok={} entry={:#x} mem={:#x}-{:#x} stack={:#x}-{:#x} arg={}",
            process.pid,
            process.main_thread.tid,
            process.parent_pid,
            process.kind,
            process.job_mode,
            process.name,
            process.state,
            process.main_thread.state,
            process.exit_status,
            process.waiting_for,
            process.context_switches,
            runtime_ticks_snapshot(process, crate::timer::ticks()),
            process.resume_context,
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
    writer.write_str("PID  TID  PPID KIND MODE STATE   EXIT WAIT RQ  CTX   TICKS NAME       ARG\n");
    let now = crate::timer::ticks();

    for process in table.processes.values() {
        writer.write_u32_padded(process.pid, 4);
        writer.write_byte(b' ');
        writer.write_u32_padded(process.main_thread.tid, 4);
        writer.write_byte(b' ');
        writer.write_opt_u32_padded(process.parent_pid, 4);
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
        writer.write_u64_padded(process.context_switches, 5);
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
        ProcessKind, ProcessState, ProcessTable, UserFaultKind, UserResumeContext,
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
        assert_eq!(table.processes.get(&pid).unwrap().main_thread.tid, pid);
        assert_eq!(
            table.processes.get(&pid).unwrap().main_thread.state,
            crate::user::thread::ThreadState::Created
        );
        table.mark_ready(pid);
        assert_eq!(
            table.processes.get(&pid).unwrap().state,
            ProcessState::Ready
        );
        assert_eq!(
            table.processes.get(&pid).unwrap().main_thread.state,
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
            table.processes.get(&pid).unwrap().main_thread.state,
            crate::user::thread::ThreadState::Zombie
        );
        assert_eq!(table.processes.get(&pid).unwrap().exit_status, Some(7));
        assert_eq!(table.stats().exited_count, 1);
        assert_eq!(table.stats().ready_queue_count, 0);
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
        assert_eq!(table.processes.get(&first).unwrap().context_switches, 1);
        assert_eq!(table.dequeue_ready(), Some(second));
        assert_eq!(table.ready_queue_position(second), None);
        assert_eq!(table.processes.get(&second).unwrap().context_switches, 1);
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
        assert_eq!(table.processes.get(&pid).unwrap().context_switches, 1);
        assert!(table.mark_yielded(pid, resume_context));
        assert_eq!(table.stats().ready_count, 1);
        assert_eq!(table.stats().ready_queue_count, 1);
        assert_eq!(table.processes.get(&pid).unwrap().last_started_tick, None);

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
        assert_eq!(table.processes.get(&pid).unwrap().context_switches, 2);
        assert!(
            table
                .processes
                .get(&pid)
                .unwrap()
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
    fn init_reaper_collects_kernel_parented_user_zombies() {
        let mut table = ProcessTable::new();
        let layout = crate::user::ring3::memory_layout_for_pid(2);
        let child = table.create_process(
            Some(1),
            "sh",
            "/bin/sh",
            crate::user::program::UserProgramArg::empty(),
            AddressSpace::kernel_shared_user(0x400000, layout),
        );

        table.mark_exited(child, 0);

        assert_eq!(table.reap_orphan_zombies(), 1);
        assert_eq!(
            table.processes.get(&child).unwrap().state,
            ProcessState::Reaped
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
