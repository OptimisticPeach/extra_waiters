use parking_lot_core::{
    park, unpark_all, unpark_one, ParkResult, UnparkResult, UnparkToken, DEFAULT_PARK_TOKEN,
    DEFAULT_UNPARK_TOKEN,
};
use std::sync::atomic::{AtomicI16, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Offers a way to wait for a signal, ensuring that
/// the waiting thread does not miss a signal in the
/// time of action.
///
/// This works by counting the number of notifications
/// the waiter has received. When entering a workload
/// where a notification could be lost, a token is
/// acquired beforehand and then provided to the waiter
/// when waiting could start. The value seen before the
/// workload is compared to the value afterwards to see
/// if a notification was missed.
///
/// # Example
/// ```
/// # use extra_waiters::attentive_waiter::AttentiveWaiter;
/// # let waiter = AttentiveWaiter::new();
/// # struct Foo;
/// # impl Foo {
/// #     fn try_lock(&self) -> bool {
/// #          true
/// #     }
/// # }
/// # let mut items = Vec::new();
/// // Acquire a token before we ever try any item.
/// let mut token = waiter.token();
/// 'a: loop {
///     // For every item, try to acquire a lock.
///     for item in &items {
///         if item.try_lock() {
///             break 'a;
///         } else {
///             panic!();
///         }
///     }
///     // If a thread notified us about
///     // an item after we tried it,
///     // we won't miss it.
///     # waiter.notify_all();
///     # items.push(Foo);
///     waiter.wait(&mut token);
/// }
/// ```
pub struct AttentiveWaiter {
    count: AtomicUsize,
    any_waiting: AtomicI16,
}

/// Keeps track of the total number of
/// notifications an [`AttentiveWaiter`] has
/// received. When starting to wait, passing
/// a mutable reference to this will allow
/// the wait function never park the thread
/// in case we missed a notification.
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct Token(usize);

/// The result of waiting on an [`AttentiveWaiter`].
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd, Hash)]
pub enum WaitResult {
    /// This thread was successfully notified.
    Success,
    /// The waiter received a notification since the
    /// acquisition of the token, so the thread was
    /// never parked.
    EarlyRet,
    /// The wait timed out.
    TimedOut,
}

impl AttentiveWaiter {
    /// Creates a new [`AttentiveWaiter`].
    pub const fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            any_waiting: AtomicI16::new(0),
        }
    }

    /// Acquires a current token which contains the number of
    /// notifications the waiter has received thus far.
    pub fn token(&self) -> Token {
        Token(self.count.load(Ordering::Acquire))
    }

    fn wait_internal(&self, Token(token): &mut Token, timeout: Option<Instant>) -> WaitResult {
        let key = self as *const _ as usize;

        let validate = || {
            let new_token = self.count.load(Ordering::SeqCst);
            if *token < new_token {
                *token = new_token;
                false
            } else {
                self.any_waiting.fetch_add(1, Ordering::AcqRel);
                true
            }
        };

        fn before_sleep() {}

        let timed_out = |new_key, _| {
            if new_key == key {
                self.any_waiting.fetch_sub(1, Ordering::Relaxed);
            }
        };

        let result = unsafe {
            park(
                key,
                validate,
                before_sleep,
                timed_out,
                DEFAULT_PARK_TOKEN,
                timeout,
            )
        };

        match result {
            ParkResult::Unparked(_) => WaitResult::Success,
            ParkResult::TimedOut => WaitResult::TimedOut,
            ParkResult::Invalid => WaitResult::EarlyRet,
        }
    }

    /// Waits for a notification, or shortcuts in the case
    /// of a missed notification. Additionally auto-updates
    /// the token provided to it with the newest value.
    pub fn wait(&self, token: &mut Token) -> WaitResult {
        self.wait_internal(token, None)
    }

    /// Waits for a notification, shortcutting if one has
    /// been submitted since the last update to the token,
    /// or timing out if waiting goes past `instant`.
    pub fn wait_until(&self, token: &mut Token, instant: Instant) -> WaitResult {
        self.wait_internal(token, Some(instant))
    }

    /// Waits for a notification, shortcutting if one has
    /// been submitted since the last update to the token,
    /// or timing out if waiting goes over the provided
    /// `duration`.
    pub fn wait_for(&self, token: &mut Token, duration: Duration) -> WaitResult {
        self.wait_internal(token, Some(std::time::Instant::now() + duration))
    }

    /// # SAFETY:
    /// `hook` may not panic nor acquire another resource managed
    /// by a parking lot implementation. For example, you can
    /// lock a system mutex (in stdlib), but not a `parking_lot`
    /// mutex.
    #[allow(unused_unsafe)]
    unsafe fn hooked_wait_internal(
        &self,
        Token(token): &mut Token,
        hook: impl FnOnce(),
        timeout: Option<Instant>,
    ) -> WaitResult {
        let key = self as *const _ as usize;

        let validate = || {
            let new_token = self.count.load(Ordering::Relaxed);
            if *token < new_token {
                *token = new_token;
                // IMPORTANT: Order.
                // Adding one to the waiting count is important because
                // that signals other threads to acquire the parking lot
                // queue lock, which must be done after the hook is run.
                false
            } else {
                hook();
                self.any_waiting.fetch_add(1, Ordering::AcqRel);
                true
            }
        };

        fn before_sleep() {}

        fn timed_out(_: usize, _: bool) {}

        let result = unsafe {
            park(
                key,
                validate,
                before_sleep,
                timed_out,
                DEFAULT_PARK_TOKEN,
                timeout,
            )
        };

        match result {
            ParkResult::Unparked(_) => WaitResult::Success,
            ParkResult::TimedOut => WaitResult::TimedOut,
            ParkResult::Invalid => WaitResult::EarlyRet,
        }
    }

    /// Checks if a notification has occurred since the last time
    /// this thread waited, or since when the token was generated
    /// before parking the thread. Aborts the parking process if
    /// there has been a notification.
    ///
    /// Additionally calls `f` while the internal queue is locked
    /// to ensure that no threads can notify between the time of
    /// running `f` and the thread going to sleep.
    ///
    /// # SAFETY:
    /// `hook` may not panic nor acquire another resource managed
    /// by a parking lot implementation. For example, you can
    /// lock a system mutex (in stdlib), but not a `parking_lot`
    /// mutex.
    pub unsafe fn hooked_wait(&self, token: &mut Token, f: impl FnOnce()) -> WaitResult {
        self.hooked_wait_internal(token, f, None)
    }

    /// Is identical to [`hooked_wait()`], however it will timeout
    /// at `instant` if no notification is received.
    ///
    /// # SAFETY:
    /// View safety constraints on [`hooked_wait()`].
    ///
    /// [`hooked_wait()`]: Self::hooked_wait()
    pub unsafe fn hooked_wait_until(
        &self,
        token: &mut Token,
        f: impl FnOnce(),
        instant: Instant,
    ) -> WaitResult {
        self.hooked_wait_internal(token, f, Some(instant))
    }

    /// Is identical to [`hooked_wait()`], however it will timeout
    /// after `duration` if no notification is received.
    ///
    /// # SAFETY:
    /// View safety constraints on [`hooked_wait()`].
    ///
    /// [`hooked_wait()`]: Self::hooked_wait()
    pub unsafe fn hooked_wait_for(
        &self,
        token: &mut Token,
        f: impl FnOnce(),
        duration: Duration,
    ) -> WaitResult {
        self.hooked_wait_internal(token, f, Some(std::time::Instant::now() + duration))
    }

    /// Submits a notification and tries to wake up one thread.
    ///
    /// Returns whether a thread was woken up or not.
    pub fn notify_one(&self) -> bool {
        self.count.fetch_add(1, Ordering::Release);

        let key = self as *const _ as usize;
        fn callback(_result: UnparkResult) -> UnparkToken {
            DEFAULT_UNPARK_TOKEN
        }

        let unparked_threads = loop {
            let count = self.any_waiting.fetch_sub(1, Ordering::Relaxed);
            if count <= 0 {
                match self.any_waiting.compare_exchange(
                    count - 1,
                    count,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break 0,
                    Err(_) => self.any_waiting.fetch_add(1, Ordering::Relaxed),
                };
            } else {
                break unsafe { unpark_one(key, callback) }.unparked_threads;
            }
        };

        unparked_threads == 1
    }

    /// Submits a notification, and wakes up any thread
    /// waiting on this waiter.
    ///
    /// Returns the number of threads woken up.
    pub fn notify_all(&self) -> usize {
        self.count.fetch_add(1, Ordering::Release);

        let key = self as *const _ as usize;

        let unparked_threads = unsafe { unpark_all(key, DEFAULT_UNPARK_TOKEN) };

        self.any_waiting
            .fetch_sub(unparked_threads as i16, Ordering::Relaxed);
        unparked_threads
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering::{Relaxed, SeqCst};
    use crate::attentive_waiter::{AttentiveWaiter, WaitResult};

    #[test]
    fn ensure_const() {
        const WAITER_CONST: AttentiveWaiter = AttentiveWaiter::new();
        static WAITER_STATIC: AttentiveWaiter = AttentiveWaiter::new();
        let _ = (WAITER_CONST, &WAITER_STATIC);
    }

    #[test]
    fn wait() {
        let waiter = AttentiveWaiter::new();
        let mut token = waiter.token();
        assert_eq!(waiter.notify_all(), 0);
        assert_eq!(waiter.wait(&mut token), WaitResult::EarlyRet);
        assert_eq!(waiter.notify_all(), 0);
        assert_eq!(waiter.wait_until(&mut token, std::time::Instant::now()), WaitResult::EarlyRet);
        assert_eq!(waiter.notify_all(), 0);
        assert_eq!(waiter.wait_for(&mut token, std::time::Duration::from_secs(0)), WaitResult::EarlyRet);
    }

    #[test]
    fn timeout() {
        let waiter = AttentiveWaiter::new();
        let mut token = waiter.token();
        assert_eq!(waiter.wait_until(&mut token, std::time::Instant::now()), WaitResult::TimedOut);
        assert_eq!(waiter.wait_for(&mut token, std::time::Duration::from_secs(0)), WaitResult::TimedOut);
    }

    #[test]
    fn wait_hooked() {
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let waiter = Arc::new(AttentiveWaiter::new());
        let flag = Arc::new(AtomicBool::new(false));

        let handle = {
            let barrier = barrier.clone();
            let waiter = waiter.clone();
            let flag = flag.clone();

            std::thread::spawn(move || {
                let mut token = waiter.token();
                flag.compare_exchange(false, true, Relaxed, Relaxed).unwrap();
                barrier.wait();

                let result = unsafe {
                    waiter.hooked_wait(&mut token, || {
                        barrier.wait();
                        flag.compare_exchange(true, false, Relaxed, Relaxed).unwrap();
                        barrier.wait();
                    })
                };

                assert_eq!(result, WaitResult::Success);
            })
        };

        barrier.wait();
        barrier.wait();
        assert_eq!(flag.load(SeqCst), true);
        barrier.wait();
        assert_eq!(flag.load(SeqCst), false);
        assert!(waiter.notify_one());
        handle.join().unwrap();
    }
}
