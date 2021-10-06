use std::sync::atomic::{AtomicBool, Ordering};

/// A waiter which spinlocks when trying to acquire
/// a lock.
///
/// Under most circumstances this is incredibly
/// inefficient and you should stick to some other
/// kind of notifier.
pub struct Spinlock(AtomicBool);

impl Spinlock {
    /// Makes a new [`Spinlock`].
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// Acquires a lock on the guard, spinlocking
    /// until a thread gives up their lock.
    pub fn lock(&self) -> SpinlockGuard {
        while self
            .0
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
        SpinlockGuard(&self.0)
    }

    /// Attempts to acquire a lock on this waiter,
    /// failing if a lock is already acquired.
    pub fn try_lock(&self) -> Option<SpinlockGuard> {
        self
            .0
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| SpinlockGuard(&self.0))
    }
}

/// A guard on a [`Spinlock`] which automatically
/// releases after being dropped from scope.
pub struct SpinlockGuard<'a>(&'a AtomicBool);

impl<'a> Drop for SpinlockGuard<'a> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}
