pub type Tid = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadKind {
    Kernel,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Created,
    Ready,
    Running,
    Blocked,
    Exited,
    Zombie,
    Reaped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thread {
    pub tid: Tid,
    pub process_id: crate::user::process::Pid,
    pub kind: ThreadKind,
    pub state: ThreadState,
}

impl Thread {
    pub const fn main(process_id: crate::user::process::Pid, kind: ThreadKind) -> Self {
        Self {
            tid: process_id,
            process_id,
            kind,
            state: ThreadState::Created,
        }
    }

    pub const fn is_main(&self) -> bool {
        self.tid == self.process_id
    }
}

#[cfg(test)]
mod tests {
    use super::{Thread, ThreadKind, ThreadState};

    #[test_case]
    fn initial_process_thread_uses_pid_as_tid() {
        let thread = Thread::main(42, ThreadKind::User);

        assert_eq!(thread.tid, 42);
        assert_eq!(thread.process_id, 42);
        assert_eq!(thread.state, ThreadState::Created);
        assert!(thread.is_main());
    }
}
