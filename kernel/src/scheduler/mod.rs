pub mod context;
pub mod queue;
pub mod task;
pub mod test_task;

use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;

pub use queue::ReadyQueue;
pub use task::{Context, TaskControlBlock, TaskKind, TaskState};
pub use test_task::test_task_entry;

use crate::serial_println;

const DEFAULT_STACK_SIZE: usize = 8 * 1024;
const MAX_TASKS: u32 = 256;
const IDLE_TASK_ID: u32 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerStats {
    pub current_task: Option<u32>,
    pub total_tasks: u32,
    pub ready_tasks: usize,
    pub total_context_switches: u64,
    pub total_ticks: u64,
}

pub struct Scheduler {
    tasks: Mutex<BTreeMap<u32, TaskControlBlock>>,
    ready_queue: ReadyQueue,
    current_task_id: Mutex<Option<u32>>,
    next_task_id: Mutex<u32>,
    total_context_switches: Mutex<u64>,
    total_ticks: AtomicU64,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            tasks: Mutex::new(BTreeMap::new()),
            ready_queue: ReadyQueue::new(),
            current_task_id: Mutex::new(None),
            next_task_id: Mutex::new(1),
            total_context_switches: Mutex::new(0),
            total_ticks: AtomicU64::new(0),
        }
    }

    pub fn create_task(&self, entry_point: extern "C" fn() -> !) -> u32 {
        let mut next_id = self.next_task_id.lock();
        if *next_id >= MAX_TASKS {
            panic!("Too many tasks created");
        }

        let task_id = *next_id;
        *next_id += 1;

        let task = TaskControlBlock::new(task_id, entry_point, DEFAULT_STACK_SIZE);
        let mut tasks = self.tasks.lock();
        tasks.insert(task_id, task);

        self.ready_queue.enqueue(task_id);

        serial_println!("[SCHEDULER] Created task {}", task_id);
        task_id
    }

    pub fn create_idle_task(&self) -> u32 {
        let mut tasks = self.tasks.lock();
        if tasks.contains_key(&IDLE_TASK_ID) {
            return IDLE_TASK_ID;
        }

        tasks.insert(
            IDLE_TASK_ID,
            TaskControlBlock::new(IDLE_TASK_ID, idle_task_entry, DEFAULT_STACK_SIZE),
        );
        serial_println!("[SCHEDULER] Created idle task");
        IDLE_TASK_ID
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
        self.ready_queue.dequeue().or(Some(IDLE_TASK_ID))
    }

    pub fn mark_task_ready(&self, task_id: u32) {
        let mut tasks = self.tasks.lock();
        if let Some(task) = tasks.get_mut(&task_id) {
            task.state = TaskState::Ready;
        }
        drop(tasks);
        self.ready_queue.enqueue(task_id);
    }

    pub fn mark_task_running(&self, task_id: u32) {
        let mut current = self.current_task_id.lock();
        *current = Some(task_id);
        drop(current);

        let mut tasks = self.tasks.lock();
        if let Some(task) = tasks.get_mut(&task_id) {
            task.state = TaskState::Running;
        }
    }

    pub fn mark_task_blocked(&self, task_id: u32) {
        let mut tasks = self.tasks.lock();
        if let Some(task) = tasks.get_mut(&task_id) {
            task.state = TaskState::Blocked;
        }
    }

    pub fn mark_task_zombie(&self, task_id: u32, exit_code: i32) {
        let mut tasks = self.tasks.lock();
        if let Some(task) = tasks.get_mut(&task_id) {
            task.state = TaskState::Zombie;
            task.exit_code = Some(exit_code);
        }
    }

    pub fn sleep_current_until(&self, wake_at_tick: u64) {
        let current = self.current_task();
        if let Some(task_id) = current {
            if task_id == IDLE_TASK_ID {
                return;
            }

            let mut tasks = self.tasks.lock();
            if let Some(task) = tasks.get_mut(&task_id) {
                task.state = TaskState::Sleeping;
                task.wake_at_tick = Some(wake_at_tick);
            }
        }
    }

    pub fn yield_now(&self) {
        if let Some(current_id) = self.current_task() {
            if current_id != IDLE_TASK_ID {
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
    /// This is intentionally not called from the normal boot path yet; it gives
    /// the kernel a controlled switch entry point while the shell remains the
    /// primary interactive surface.
    pub unsafe fn start_first_task(&self) -> ! {
        let task_id = self.schedule().unwrap_or(IDLE_TASK_ID);
        self.mark_task_running(task_id);
        *self.total_context_switches.lock() += 1;

        let context = self
            .get_context(task_id)
            .expect("scheduled task must have a context");

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

    pub fn get_stats(&self) -> (u32, usize) {
        let tasks = self.tasks.lock();
        (tasks.len() as u32, self.ready_queue.len())
    }

    pub fn stats(&self) -> SchedulerStats {
        let tasks = self.tasks.lock();
        SchedulerStats {
            current_task: *self.current_task_id.lock(),
            total_tasks: tasks.len() as u32,
            ready_tasks: self.ready_queue.len(),
            total_context_switches: *self.total_context_switches.lock(),
            total_ticks: self.total_ticks.load(Ordering::Relaxed),
        }
    }

    pub fn task_runtime_ticks(&self, task_id: u32) -> Option<u64> {
        self.tasks
            .lock()
            .get(&task_id)
            .map(|task| task.runtime_ticks)
    }

    fn wake_sleeping_tasks(&self) {
        self.wake_sleeping_tasks_at(crate::timer::ticks());
    }

    fn wake_sleeping_tasks_at(&self, tick: u64) {
        let mut woke_tasks = alloc::vec::Vec::new();
        {
            let mut tasks = self.tasks.lock();
            for (task_id, task) in tasks.iter_mut() {
                if task.state == TaskState::Sleeping && task.wake_at_tick <= Some(tick) {
                    task.state = TaskState::Ready;
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
