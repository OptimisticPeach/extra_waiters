use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;
use std::time::Instant;
use crossbeam_channel::{Sender, Receiver, unbounded};

pub struct AsyncToken(usize);

pub enum AsyncWaitResult {
    EarlyReturn,
    Waited,
    Timeout,
}

pub struct Waiter {
    sender: Sender<Weak<Waker>>,
    receiver: Receiver<Weak<Waker>>,
    notif_count: AtomicUsize,
}

impl Waiter {
    pub fn new() -> Self {
        let (send, recv) = unbounded();

        Self {
            sender: send,
            receiver: recv,
            notif_count: AtomicUsize::new(1),
        }
    }

    pub fn wait<'a, 'b>(&'a self, AsyncToken(token): &'b mut AsyncToken) -> WaiterFuture<'a, 'b> {
        WaiterFuture {
            waiter: self,
            token,
            awoken: AtomicU8::new(PENDING),
            waker: None,
        }
    }

    pub fn notify_one(&self) -> bool {
        // When adding a waker, we push waker then check number.
        // We must do opposite order here:
        self
            .notif_count
            .fetch_add(1, Ordering::Relaxed);

        self
            .receiver
            .try_iter()
            .any(|x| {
                x
                    .upgrade()
                    .map(|waker| waker.wake_by_ref())
                    .is_some()
            })
    }

    pub fn notify_all(&self) -> usize {
        // When adding a waker, we push waker then check number.
        // We must do opposite order here:
        self
            .notif_count
            .fetch_add(1, Ordering::Relaxed);

        self
            .receiver
            .try_iter()
            .filter(|x| {
                x
                    .upgrade()
                    .map(|waker| waker.wake_by_ref())
                    .is_some()
            })
            .count()
    }
}

const PENDING: u8 = 0;
const NOT_AWOKEN: u8 = 1;
const AWOKEN: u8 = 2;
const EARLY_AWAKE: u8 = 4;

pub struct WaiterFuture<'a, 'b> {
    waiter: &'a Waiter,
    token: &'b mut usize,
    awoken: AtomicU8,
    waker: Option<Arc<Waker>>,
}

impl<'a, 'b> Future for WaiterFuture<'a, 'b> {
    type Output = AsyncWaitResult;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // todo: timeout wake
        let state = self.awoken.load(Ordering::Relaxed);
        let notif_count = self.waiter.notif_count.load(Ordering::Relaxed);

        match state {
            PENDING => if notif_count == *self.token {
                self
                    .awoken
                    .compare_exchange(
                        PENDING,
                        NOT_AWOKEN,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .unwrap();

                let waker = Arc::new(cx.waker().clone());

                self
                    .waiter
                    .sender
                    .send(
                        Arc::downgrade(&waker)
                    )
                    .unwrap();

                if self.waiter.notif_count.load(Ordering::SeqCst) == *self.token {
                    self.waker.insert(waker);
                    Poll::Pending
                } else {
                    self
                        .awoken
                        .compare_exchange(
                            NOT_AWOKEN,
                            AWOKEN,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        )
                        .unwrap();

                    drop(waker);

                    Poll::Ready(AsyncWaitResult::Waited)
                }
            } else {
                self
                    .awoken
                    .compare_exchange(
                        PENDING,
                        EARLY_AWAKE,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .unwrap();
                Poll::Ready(AsyncWaitResult::EarlyReturn)
            },
            NOT_AWOKEN => {
                if self.waiter.notif_count.load(Ordering::SeqCst) == *self.token {
                    Poll::Pending
                } else {
                    self
                        .awoken
                        .compare_exchange(
                            NOT_AWOKEN,
                            AWOKEN,
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                        )
                        .unwrap();

                    drop(self.waker.take().unwrap());

                    Poll::Ready(AsyncWaitResult::Waited)
                }
            },
            AWOKEN => {
                Poll::Ready(AsyncWaitResult::Waited)
            },
            EARLY_AWAKE => {
                Poll::Ready(AsyncWaitResult::EarlyReturn)
            }
            _ => unreachable!(),
        }
    }
}
