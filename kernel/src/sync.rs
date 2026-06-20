use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use spin::{Mutex, MutexGuard};
use x86_64::instructions::interrupts;

static PREEMPT_DISABLE_DEPTH: AtomicUsize = AtomicUsize::new(0);
static PREEMPT_DEFERRED: AtomicBool = AtomicBool::new(false);

pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    interrupts::without_interrupts(f)
}

pub fn preemption_disabled() -> bool {
    PREEMPT_DISABLE_DEPTH.load(Ordering::Acquire) != 0
}

pub fn defer_preemption() {
    PREEMPT_DEFERRED.store(true, Ordering::Release);
}

pub fn take_deferred_preemption() -> bool {
    PREEMPT_DEFERRED.swap(false, Ordering::AcqRel)
}

pub struct PreemptionGuard;

impl PreemptionGuard {
    pub fn new() -> Self {
        PREEMPT_DISABLE_DEPTH.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for PreemptionGuard {
    fn drop(&mut self) {
        let previous = PREEMPT_DISABLE_DEPTH.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "unbalanced preemption guard");
    }
}

pub struct PreemptMutex<T: ?Sized> {
    inner: Mutex<T>,
}

impl<T> PreemptMutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(value),
        }
    }
}

impl<T: ?Sized> PreemptMutex<T> {
    pub fn lock(&self) -> PreemptMutexGuard<'_, T> {
        let interrupts = InterruptGuard::new();
        let preemption = PreemptionGuard::new();
        let inner = self.inner.lock();
        PreemptMutexGuard {
            inner,
            _preemption: preemption,
            _interrupts: interrupts,
        }
    }
}

pub struct PreemptMutexGuard<'a, T: ?Sized> {
    inner: MutexGuard<'a, T>,
    _preemption: PreemptionGuard,
    _interrupts: InterruptGuard,
}

impl<T: ?Sized> Deref for PreemptMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: ?Sized> DerefMut for PreemptMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

pub struct InterruptGuard {
    was_enabled: bool,
}

impl InterruptGuard {
    pub fn new() -> Self {
        let was_enabled = interrupts::are_enabled();
        interrupts::disable();
        Self { was_enabled }
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        if self.was_enabled {
            interrupts::enable();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PreemptMutex, PreemptionGuard, preemption_disabled};

    #[test_case]
    fn preemption_guard_is_nestable() {
        assert!(!preemption_disabled());
        let outer = PreemptionGuard::new();
        assert!(preemption_disabled());
        {
            let _inner = PreemptionGuard::new();
            assert!(preemption_disabled());
        }
        assert!(preemption_disabled());
        drop(outer);
        assert!(!preemption_disabled());
    }

    #[test_case]
    fn preempt_mutex_guards_critical_section() {
        let value = PreemptMutex::new(7_u64);
        {
            let mut guard = value.lock();
            assert!(preemption_disabled());
            *guard += 1;
        }
        assert!(!preemption_disabled());
        assert_eq!(*value.lock(), 8);
    }
}
