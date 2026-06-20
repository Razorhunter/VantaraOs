use crate::sync::PreemptMutex as Mutex;
use alloc::collections::VecDeque;

pub struct ReadyQueue {
    queue: Mutex<VecDeque<u32>>,
}

impl ReadyQueue {
    pub const fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
        }
    }

    pub fn enqueue(&self, task_id: u32) {
        self.queue.lock().push_back(task_id);
    }

    pub fn dequeue(&self) -> Option<u32> {
        self.queue.lock().pop_front()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.lock().is_empty()
    }

    pub fn len(&self) -> usize {
        self.queue.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::ReadyQueue;

    #[test_case]
    fn ready_queue_fifo_order() {
        let queue = ReadyQueue::new();

        queue.enqueue(1);
        queue.enqueue(2);
        queue.enqueue(3);

        assert_eq!(queue.len(), 3);
        assert_eq!(queue.dequeue(), Some(1));
        assert_eq!(queue.dequeue(), Some(2));
        assert_eq!(queue.dequeue(), Some(3));
        assert_eq!(queue.dequeue(), None);
    }
}
