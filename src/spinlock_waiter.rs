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

    /// Samples whether the lock is currently acquired.
    ///
    /// This will panic if the ordering is `Release` or `AcqRel`.
    pub fn is_locked(&self, ordering: Ordering) -> bool {
        self
            .0
            .load(ordering)
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use crate::spinlock_waiter::Spinlock;

    #[test]
    fn create() {
        const X: Spinlock = Spinlock::new();
        let _ = X;
    }

    #[test]
    fn lock() {
        let lock = Spinlock::new();
        let _x = lock.lock();
        assert!(lock.is_locked(Ordering::SeqCst));
        assert!(matches!(lock.try_lock(), None));
    }

    #[test]
    fn unlock() {
        let lock = Spinlock::new();


        let guard = lock.lock();
        assert!(lock.is_locked(Ordering::SeqCst));
        drop(guard);


        assert!(!lock.is_locked(Ordering::SeqCst));

        assert!(matches!(lock.try_lock(), Some(_)));
    }
}
