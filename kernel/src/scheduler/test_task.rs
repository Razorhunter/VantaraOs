use core::sync::atomic::{AtomicU32, Ordering};

static TASK_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

/// Simple kernel-mode test task that prints its task ID and iteration count
/// This function serves as an entry point for the scheduler to demonstrate
/// task execution and context switching.
pub fn test_task_entry() {
    let task_id = TASK_ID_COUNTER.fetch_add(1, Ordering::SeqCst);

    for iteration in 0..10 {
        crate::serial_println!("[Task {}] Running iteration {}", task_id, iteration);

        // Simulate some work with a simple busy loop
        for _ in 0..1_000_000 {
            core::hint::spin_loop();
        }
    }

    crate::serial_println!("[Task {}] Task completed", task_id);
}
