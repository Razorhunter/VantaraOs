use alloc::vec::Vec;

pub type Tid = u32;
pub const MAX_THREADS_PER_PROCESS: usize = 16;

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
    Stopped,
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
    pub resume_context: Option<crate::user::process::UserResumeContext>,
    pub signal_return_context: Option<crate::user::process::UserResumeContext>,
    pub context_switches: u64,
    pub runtime_ticks: u64,
    pub last_started_tick: Option<u64>,
    pub user_stack_start: Option<u64>,
    pub user_stack_top: Option<u64>,
    pub kernel_managed_stack: bool,
    pub waiting_for: Option<Tid>,
}

impl Thread {
    pub const fn new(tid: Tid, process_id: crate::user::process::Pid, kind: ThreadKind) -> Self {
        Self {
            tid,
            process_id,
            kind,
            state: ThreadState::Created,
            resume_context: None,
            signal_return_context: None,
            context_switches: 0,
            runtime_ticks: 0,
            last_started_tick: None,
            user_stack_start: None,
            user_stack_top: None,
            kernel_managed_stack: false,
            waiting_for: None,
        }
    }

    pub const fn with_user_stack(mut self, stack_start: u64, stack_top: u64) -> Self {
        self.user_stack_start = Some(stack_start);
        self.user_stack_top = Some(stack_top);
        self
    }

    pub const fn with_kernel_managed_stack(mut self) -> Self {
        self.kernel_managed_stack = true;
        self
    }
}

#[derive(Debug)]
pub struct ThreadStore {
    main_tid: Tid,
    items: Vec<Thread>,
}

impl ThreadStore {
    pub fn with_main(main: Thread) -> Self {
        Self {
            main_tid: main.tid,
            items: alloc::vec![main],
        }
    }

    pub fn main_tid(&self) -> Tid {
        self.main_tid
    }

    pub fn main(&self) -> &Thread {
        self.get(self.main_tid)
            .expect("thread store must contain its main thread")
    }

    pub fn main_mut(&mut self) -> &mut Thread {
        self.get_mut(self.main_tid)
            .expect("thread store must contain its main thread")
    }

    pub fn get(&self, tid: Tid) -> Option<&Thread> {
        self.items.iter().find(|thread| thread.tid == tid)
    }

    pub fn get_mut(&mut self, tid: Tid) -> Option<&mut Thread> {
        self.items.iter_mut().find(|thread| thread.tid == tid)
    }

    pub fn insert(&mut self, thread: Thread) -> bool {
        if self.items.len() >= MAX_THREADS_PER_PROCESS
            || thread.process_id != self.main().process_id
            || self.get(thread.tid).is_some()
        {
            return false;
        }
        self.items.push(thread);
        true
    }

    pub fn stack_range_available(&self, stack_start: u64, stack_top: u64) -> bool {
        stack_start < stack_top
            && self.items.iter().all(|thread| {
                let (Some(existing_start), Some(existing_top)) =
                    (thread.user_stack_start, thread.user_stack_top)
                else {
                    return true;
                };
                stack_top <= existing_start || stack_start >= existing_top
            })
    }

    pub fn values(&self) -> core::slice::Iter<'_, Thread> {
        self.items.iter()
    }

    pub fn values_mut(&mut self) -> core::slice::IterMut<'_, Thread> {
        self.items.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{Thread, ThreadKind, ThreadState, ThreadStore};

    #[test_case]
    fn thread_identity_is_independent_from_process_identity() {
        let thread = Thread::new(84, 42, ThreadKind::User);

        assert_eq!(thread.tid, 84);
        assert_eq!(thread.process_id, 42);
        assert_eq!(thread.state, ThreadState::Created);
        assert_eq!(thread.user_stack_top, None);
    }

    #[test_case]
    fn thread_store_tracks_main_and_additional_threads() {
        let mut threads = ThreadStore::with_main(Thread::new(7, 42, ThreadKind::User));

        assert_eq!(threads.main_tid(), 7);
        assert_eq!(threads.main().process_id, 42);
        assert!(threads.insert(Thread::new(8, 42, ThreadKind::User)));
        assert!(!threads.insert(Thread::new(8, 42, ThreadKind::User)));
        assert!(!threads.insert(Thread::new(9, 99, ThreadKind::User)));
        assert_eq!(threads.len(), 2);
    }

    #[test_case]
    fn thread_store_rejects_overlapping_user_stack_ranges() {
        let threads = ThreadStore::with_main(
            Thread::new(7, 42, ThreadKind::User).with_user_stack(0x7000, 0x8000),
        );

        assert!(threads.stack_range_available(0x6000, 0x7000));
        assert!(!threads.stack_range_available(0x7800, 0x8800));
    }
}
