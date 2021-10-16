use parking_lot_core::{park, unpark_all, DEFAULT_PARK_TOKEN, DEFAULT_UNPARK_TOKEN, ParkResult};
use std::any::Any;
use std::panic::{catch_unwind, UnwindSafe};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Instant, Duration};

const DEFAULT: u8 = 0b00;
const PREPPING: u8 = 0b01;
const READY: u8 = 0b10;

/// Waits until one thread has successfully
/// finished its initialization.
pub struct OnceWaiter {
    state: AtomicU8,
}

impl OnceWaiter {
    /// Creates a new [`OnceWaiter`], in an uninitialized
    /// state.
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(DEFAULT),
        }
    }

    fn finish_internal(
        &self,
        f: impl FnOnce() + UnwindSafe,
        timeout: Option<Instant>,
    ) -> Result<usize, Box<dyn Any + Send + 'static>> {
        let key = self as *const _ as usize;
        loop {
            match self.state.compare_exchange(
                DEFAULT,
                PREPPING,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let result = catch_unwind(f);
                    match &result {
                        Ok(_) => self.state.store(READY, Ordering::Relaxed),
                        Err(_) => self.state.store(DEFAULT, Ordering::Relaxed),
                    }

                    let woken = unsafe { unpark_all(key, DEFAULT_UNPARK_TOKEN) };

                    break result.map(|_| woken);
                }
                Err(val) => {
                    if val == PREPPING {
                        let result = unsafe {
                            let validate = || self.state.load(Ordering::Relaxed) == PREPPING;
                            fn before_sleep() {}
                            fn timed_out(_: usize, _: bool) {}
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
                            ParkResult::TimedOut => Err(Box::new("Once waiter timed out") as Box<dyn Any + Send>)?,
                            ParkResult::Invalid => continue,
                            ParkResult::Unparked(_) => continue,
                        }
                    } else if val == READY {
                        break Ok(0);
                    }
                }
            }
        }
    }

    /// Attempts to finish this waiter.
    ///
    /// Only one thread can try to initialize this value
    /// at a time.
    ///
    /// If another thread is trying to initialize this waiter,
    /// it will wait until it produces a result. If the other
    /// thread fails to initialize (by panicking), then the
    /// waiter is unlocked, and only one of the threads waiting
    /// to initialize is permitted to run.
    ///
    /// # Returns
    /// This returns the number of threads which it awoke if
    /// it succeeds or 0 if another thread initializes it
    /// before this thread does. In case of the function
    /// panicking, it will return the boxed panic error.
    ///
    /// # Variants
    /// * For a version which waits until an instant in time,
    ///   see [`try_finish_by()`].
    /// * For a version which waits for a specified duration,
    ///   see [`try_finish_after()`].
    ///
    ///
    /// [`try_finish_by()`]: Self::try_finish_by
    /// [`try_finish_after()`]: Self::try_finish_after
    pub fn finish<F: FnOnce() + UnwindSafe>(&self, f: F) -> Result<usize, Box<dyn Any + Send + 'static>> {
        self.finish_internal(f, None)
    }

    /// Attempts to finish this waiter by an instant in time.
    ///
    /// This performs the same action as [`finish()`], but will
    /// give up if it cannot run its function before the `instant`
    /// provided.
    ///
    /// For more details and return value, see [`finish()`].
    ///
    /// [`finish()`]: Self::finish
    pub fn try_finish_by<F: FnOnce() + UnwindSafe>(&self, f: F, instant: Instant) -> Result<usize, Box<dyn Any + Send + 'static>> {
        self.finish_internal(f, Some(instant))
    }

    /// Attempts to finish this waiter after a given duration.
    ///
    /// This performs the same action as [`finish()`], but will
    /// give up if it cannot run its function after `duration`
    /// has passed.
    ///
    /// For more details and return value, see [`finish()`].
    ///
    /// [`finish()`]: Self::finish
    pub fn try_finish_after<F: FnOnce() + UnwindSafe>(&self, f: F, duration: Duration) -> Result<usize, Box<dyn Any + Send + 'static>> {
        self.finish_internal(f, Some(Instant::now() + duration))
    }

    fn wait_until_done_internal(&self, timeout: Option<Instant>) -> Result<(), ()> {
        let key = self as *const _  as usize;
        loop {
            match self.state.load(Ordering::Relaxed) {
                PREPPING | DEFAULT => {
                    let result = unsafe {
                        let validate = || self.state.load(Ordering::Relaxed) != READY;
                        fn before_sleep() {}
                        fn timed_out(_: usize, _: bool) {}
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
                        ParkResult::Invalid => break Ok(()),
                        ParkResult::TimedOut => if self.state.load(Ordering::Relaxed) == READY {
                            break Ok(())
                        } else {
                            break Err(())
                        },
                        ParkResult::Unparked(_) => continue,
                    }
                },
                READY => break Ok(()),
                _ => unreachable!(),
            }
        }
    }

    pub fn wait_until_done(&self) {
        self.wait_until_done_internal(None).unwrap();
    }

    pub fn wait_until_done_until(&self, instant: Instant) -> Result<(), ()> {
        self.wait_until_done_internal(Some(instant))
    }

    pub fn wait_until_done_for(&self, duration: Duration) -> Result<(), ()> {
        self.wait_until_done_internal(Some(Instant::now() + duration))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::sync::atomic::{AtomicU8, Ordering};
    use crate::once_waiter::OnceWaiter;

    #[test]
    fn create() {
        const X: OnceWaiter = OnceWaiter::new();

        let _ = X;
    }

    #[test]
    fn ensure_once() {
        for _ in 0..100 {
            let barrier = Arc::new(Barrier::new(4));

            let flag = Arc::new(AtomicU8::new(0));

            let waiter = Arc::new(OnceWaiter::new());

            let handles = (0..3)
                .map(|_| {
                    let barrier = barrier.clone();
                    let flag = flag.clone();
                    let waiter = waiter.clone();

                    std::thread::spawn(move || {
                        barrier.wait();

                        waiter.finish(|| {
                            flag.fetch_add(1, Ordering::SeqCst);
                        })
                            .unwrap();
                    })
                })
                .collect::<Vec<_>>();

            barrier.wait();
            waiter.finish(|| {
                flag.fetch_add(1, Ordering::SeqCst);
            })
                .unwrap();

            handles
                .into_iter()
                .for_each(|x| x.join().unwrap());

            assert_eq!(flag.load(Ordering::SeqCst), 1);
        }
    }
}
