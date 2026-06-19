use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,
    Sleeping,
    Zombie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    Kernel,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Context {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub rip: u64,
    pub rflags: u64,
    pub rsp: u64,
}

const _: () = {
    assert!(core::mem::size_of::<Context>() == 144);
    assert!(core::mem::offset_of!(Context, r15) == 0x00);
    assert!(core::mem::offset_of!(Context, rax) == 0x70);
    assert!(core::mem::offset_of!(Context, rip) == 0x78);
    assert!(core::mem::offset_of!(Context, rflags) == 0x80);
    assert!(core::mem::offset_of!(Context, rsp) == 0x88);
};

impl Context {
    pub fn new() -> Self {
        Self {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rbp: 0,
            rdi: 0,
            rsi: 0,
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            rip: 0,
            rflags: 0x200, // IF bit set (interrupts enabled)
            rsp: 0,
        }
    }

    pub fn with_entry_point(entry: u64, stack_top: u64) -> Self {
        let mut ctx = Self::new();
        ctx.rip = entry;
        ctx.rsp = stack_top;
        ctx
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Context")
            .field("rip", &self.rip)
            .field("rsp", &self.rsp)
            .field("rax", &self.rax)
            .field("rbx", &self.rbx)
            .finish()
    }
}

pub struct TaskControlBlock {
    pub id: u32,
    pub kind: TaskKind,
    pub owner_process: Option<crate::user::process::Pid>,
    pub state: TaskState,
    pub priority: u8,
    pub context: Context,
    pub stack: alloc::vec::Vec<u8>,
    pub created_at: u64,
    pub cpu_time_ms: u64,
    pub runtime_ticks: u64,
    pub wake_at_tick: Option<u64>,
    pub exit_code: Option<i32>,
}

impl TaskControlBlock {
    pub fn new(id: u32, entry_point: extern "C" fn() -> !, stack_size: usize) -> Self {
        let mut stack = alloc::vec::Vec::with_capacity(stack_size);
        stack.resize(stack_size, 0);
        // A C-ABI function observes RSP % 16 == 8 at entry because a normal
        // `call` pushes an 8-byte return address. The bootstrap uses `ret` to
        // enter the task, so reserve the equivalent slot explicitly.
        let stack_top = ((stack.as_ptr() as u64 + stack_size as u64) & !0xf).saturating_sub(8);

        Self {
            id,
            kind: TaskKind::Kernel,
            owner_process: None,
            state: TaskState::Ready,
            priority: 0,
            context: Context::with_entry_point(entry_point as *const () as u64, stack_top),
            stack,
            created_at: 0,
            cpu_time_ms: 0,
            runtime_ticks: 0,
            wake_at_tick: None,
            exit_code: None,
        }
    }

    pub fn is_runnable(&self) -> bool {
        self.state == TaskState::Ready || self.state == TaskState::Running
    }
}

impl fmt::Debug for TaskControlBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("TaskControlBlock")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("owner_process", &self.owner_process)
            .field("state", &self.state)
            .field("priority", &self.priority)
            .field("cpu_time_ms", &self.cpu_time_ms)
            .field("runtime_ticks", &self.runtime_ticks)
            .field("exit_code", &self.exit_code)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{TaskControlBlock, TaskKind, TaskState};

    extern "C" fn test_entry() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[test_case]
    fn new_task_starts_ready_with_aligned_stack() {
        let task = TaskControlBlock::new(1, test_entry, 4096);

        assert_eq!(task.state, TaskState::Ready);
        assert_eq!(task.kind, TaskKind::Kernel);
        assert_eq!(task.owner_process, None);
        assert_eq!(task.context.rsp % 16, 8);
        assert!(task.is_runnable());
    }

    #[test_case]
    fn sleeping_task_is_not_runnable() {
        let mut task = TaskControlBlock::new(2, test_entry, 4096);
        task.state = TaskState::Sleeping;

        assert!(!task.is_runnable());
    }
}
