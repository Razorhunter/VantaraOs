pub mod context;
pub mod queue;
pub mod task;
pub mod test_task;

use crate::sync::PreemptMutex as Mutex;
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub use queue::ReadyQueue;
pub use task::{Context, TaskControlBlock, TaskKind};
pub use test_task::test_task_entry;

use crate::serial_println;

const DEFAULT_STACK_SIZE: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerStats {
    pub current_task: Option<u32>,
    pub idle_thread: Option<u32>,
    pub total_tasks: u32,
    pub ready_tasks: usize,
    pub total_context_switches: u64,
    pub total_ticks: u64,
}

pub struct Scheduler {
    tasks: Mutex<BTreeMap<u32, TaskControlBlock>>,
    ready_queue: ReadyQueue,
    current_task_id: Mutex<Option<u32>>,
    idle_task_id: Mutex<Option<u32>>,
    total_context_switches: Mutex<u64>,
    total_ticks: AtomicU64,
    live_switching: AtomicBool,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            tasks: Mutex::new(BTreeMap::new()),
            ready_queue: ReadyQueue::new(),
            current_task_id: Mutex::new(None),
            idle_task_id: Mutex::new(None),
            total_context_switches: Mutex::new(0),
            total_ticks: AtomicU64::new(0),
            live_switching: AtomicBool::new(false),
        }
    }

    pub fn create_task(&self, entry_point: extern "C" fn() -> !) -> u32 {
        let task_id = crate::user::process::create_kernel_thread()
            .expect("kernel process must exist before creating scheduler tasks");
        let task = TaskControlBlock::new(task_id, 1, entry_point, DEFAULT_STACK_SIZE);
        let mut tasks = self.tasks.lock();
        tasks.insert(task_id, task);

        self.ready_queue.enqueue(task_id);

        serial_println!("[SCHEDULER] Created task {}", task_id);
        task_id
    }

    pub fn create_idle_task(&self) -> u32 {
        if let Some(idle_tid) = *self.idle_task_id.lock() {
            return idle_tid;
        }

        let idle_tid = crate::user::process::create_kernel_thread()
            .expect("kernel process must exist before creating idle thread");
        let mut tasks = self.tasks.lock();
        tasks.insert(
            idle_tid,
            TaskControlBlock::new(idle_tid, 1, idle_task_entry, DEFAULT_STACK_SIZE),
        );
        *self.idle_task_id.lock() = Some(idle_tid);
        serial_println!("[SCHEDULER] Created idle task");
        idle_tid
    }

    pub fn current_task(&self) -> Option<u32> {
        *self.current_task_id.lock()
    }

    pub fn get_task(&self, task_id: u32) -> Option<u32> {
        let tasks = self.tasks.lock();
        if tasks.contains_key(&task_id) {
            Some(task_id)
        } else {
            None
        }
    }

    pub fn schedule(&self) -> Option<u32> {
        self.wake_sleeping_tasks();
        self.ready_queue
            .dequeue()
            .or_else(|| *self.idle_task_id.lock())
    }

    pub fn mark_task_ready(&self, task_id: u32) {
        crate::user::process::set_kernel_thread_state(
            task_id,
            crate::user::thread::ThreadState::Ready,
        );
        self.ready_queue.enqueue(task_id);
    }

    pub fn mark_task_running(&self, task_id: u32) {
        let mut current = self.current_task_id.lock();
        *current = Some(task_id);
        drop(current);

        crate::user::process::set_kernel_thread_state(
            task_id,
            crate::user::thread::ThreadState::Running,
        );
    }

    pub fn mark_task_blocked(&self, task_id: u32) {
        crate::user::process::set_kernel_thread_state(
            task_id,
            crate::user::thread::ThreadState::Blocked,
        );
    }

    pub fn mark_task_zombie(&self, task_id: u32, exit_code: i32) {
        let mut tasks = self.tasks.lock();
        if let Some(task) = tasks.get_mut(&task_id) {
            task.exit_code = Some(exit_code);
        }
        crate::user::process::set_kernel_thread_state(
            task_id,
            crate::user::thread::ThreadState::Zombie,
        );
    }

    pub fn sleep_current_until(&self, wake_at_tick: u64) {
        let current = self.current_task();
        if let Some(task_id) = current {
            if Some(task_id) == *self.idle_task_id.lock() {
                return;
            }

            let mut tasks = self.tasks.lock();
            if let Some(task) = tasks.get_mut(&task_id) {
                task.wake_at_tick = Some(wake_at_tick);
            }
            crate::user::process::set_kernel_thread_state(
                task_id,
                crate::user::thread::ThreadState::Blocked,
            );
        }
    }

    pub fn yield_now(&self) {
        if let Some(current_id) = self.current_task() {
            if Some(current_id) != *self.idle_task_id.lock() {
                self.mark_task_ready(current_id);
            }
        }

        if let Some(next_id) = self.schedule() {
            self.mark_task_running(next_id);
            *self.total_context_switches.lock() += 1;
        }
    }

    /// Starts the first scheduled task and never returns.
    ///
    /// Normal boot calls this after subsystem initialization to leave the
    /// temporary boot stack and enter the scheduler-owned runtime thread.
    pub unsafe fn start_first_task(&self) -> ! {
        let task_id = self
            .schedule()
            .expect("idle kernel thread must exist before scheduler start");
        self.mark_task_running(task_id);
        *self.total_context_switches.lock() += 1;

        let frame_rsp = self
            .saved_interrupt_frame(task_id)
            .expect("scheduled task must have an interrupt frame");
        let context = self
            .get_context(task_id)
            .expect("scheduled task must have a bootstrap context");
        if let Some((stack_start, stack_end)) = self.task_stack_range(task_id) {
            serial_println!(
                "[KTHREAD] bootstrap tid={} frame={:#x} stack={:#x}-{:#x}",
                task_id,
                frame_rsp,
                stack_start,
                stack_end
            );
        }
        if let Some(task) = self.tasks.lock().get_mut(&task_id) {
            task.has_started = true;
        }
        self.live_switching.store(true, Ordering::Release);
        crate::interrupts::disable();

        unsafe {
            context::setup_first_task(&context);
        }
    }

    pub fn tick(&self) {
        self.total_ticks.fetch_add(1, Ordering::Relaxed);
    }

    pub fn preempt_current(&self) -> Option<(u32, u32)> {
        let current = self.current_task_id.lock();

        if let Some(current_id) = *current {
            drop(current);

            if let Some(next_id) = self.schedule() {
                self.mark_task_ready(current_id);
                self.mark_task_running(next_id);
                *self.total_context_switches.lock() += 1;

                serial_println!(
                    "[SCHEDULER] Context switch: Task {} -> Task {}",
                    current_id,
                    next_id
                );
                return Some((current_id, next_id));
            }
        }
        None
    }

    pub fn preempt_from_timer(&self, current_frame_rsp: u64) -> Option<u64> {
        if !self.live_switching.load(Ordering::Acquire) {
            return None;
        }
        if crate::sync::preemption_disabled() {
            crate::sync::defer_preemption();
            return None;
        }
        let _was_deferred = crate::sync::take_deferred_preemption();

        let current_id = self.current_task()?;
        {
            let mut tasks = self.tasks.lock();
            tasks.get_mut(&current_id)?.saved_interrupt_frame = current_frame_rsp;
        }

        if Some(current_id) != *self.idle_task_id.lock() {
            self.mark_task_ready(current_id);
        }
        let next_id = self.schedule()?;
        self.mark_task_running(next_id);
        if next_id == current_id {
            return None;
        }
        *self.total_context_switches.lock() += 1;
        let mut tasks = self.tasks.lock();
        let next = tasks.get_mut(&next_id)?;
        let fresh_entry = !next.has_started;
        next.has_started = true;
        // Kernel stack/frame pointers are at least 8-byte aligned. Bit zero is
        // therefore free for the assembly return path to distinguish a fresh
        // entry frame from a CPU-produced interrupt frame that needs iretq.
        Some(next.saved_interrupt_frame | u64::from(fresh_entry))
    }

    pub fn get_context(&self, task_id: u32) -> Option<Context> {
        let tasks = self.tasks.lock();
        tasks.get(&task_id).map(|t| t.context)
    }

    pub fn update_context(&self, task_id: u32, context: Context) {
        let mut tasks = self.tasks.lock();
        if let Some(task) = tasks.get_mut(&task_id) {
            task.context = context;
        }
    }

    pub fn saved_interrupt_frame(&self, task_id: u32) -> Option<u64> {
        self.tasks
            .lock()
            .get(&task_id)
            .map(|task| task.saved_interrupt_frame)
    }

    fn task_stack_range(&self, task_id: u32) -> Option<(u64, u64)> {
        let tasks = self.tasks.lock();
        let task = tasks.get(&task_id)?;
        let start = task.stack.as_ptr() as u64;
        Some((start, start + task.stack.len() as u64))
    }

    pub fn get_stats(&self) -> (u32, usize) {
        let tasks = self.tasks.lock();
        (tasks.len() as u32, self.ready_queue.len())
    }

    pub fn stats(&self) -> SchedulerStats {
        let tasks = self.tasks.lock();
        SchedulerStats {
            current_task: *self.current_task_id.lock(),
            idle_thread: *self.idle_task_id.lock(),
            total_tasks: tasks.len() as u32,
            ready_tasks: self.ready_queue.len(),
            total_context_switches: *self.total_context_switches.lock(),
            total_ticks: self.total_ticks.load(Ordering::Relaxed),
        }
    }

    pub fn task_runtime_ticks(&self, task_id: u32) -> Option<u64> {
        crate::user::process::kernel_thread_runtime_ticks(task_id)
    }

    fn wake_sleeping_tasks(&self) {
        self.wake_sleeping_tasks_at(crate::timer::ticks());
    }

    fn wake_sleeping_tasks_at(&self, tick: u64) {
        let mut woke_tasks = alloc::vec::Vec::new();
        {
            let mut tasks = self.tasks.lock();
            for (task_id, task) in tasks.iter_mut() {
                if crate::user::process::kernel_thread_state(*task_id)
                    == Some(crate::user::thread::ThreadState::Blocked)
                    && task.wake_at_tick <= Some(tick)
                {
                    task.wake_at_tick = None;
                    woke_tasks.push(*task_id);
                }
            }
        }

        for task_id in woke_tasks {
            self.ready_queue.enqueue(task_id);
        }
    }
}

lazy_static::lazy_static! {
    pub static ref SCHEDULER: Scheduler = Scheduler::new();
}

pub fn init() {
    serial_println!("[SCHEDULER] Initializing scheduler...");
}

pub fn yield_now() {
    SCHEDULER.yield_now();
}

extern "C" fn idle_task_entry() -> ! {
    loop {
        crate::hlt_loop_once();
    }
}
