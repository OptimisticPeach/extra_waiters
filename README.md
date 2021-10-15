# `extra_waiters`

This library provides synchronization primitives built on
top of `parking_lot_core`. The various waiters here are a
mix between a `Condvar` and a `Mutex`, with the specific
details being particular to each waiter.

Examples of usage can be seen in the test cases written
for each module, however the general pattern for each of
the waiters is as follows:

## [`AttentiveWaiter`]

[`AttentiveWaiter`]: attentive_waiter::AttentiveWaiter

This waiter provides functions to do the following:

* Not wait for a notification, should one have occurred
  before the last time a notification was polled for.

* Execute some code before the thread sleeps in a way
  that no thread could notify between the time the
  user code runs and the thread sleeps.

* Perform the previous two bullet points using timeout
  functions which timeout at a certain `Instant` in time,
  or after a certain `Duration`.

## [`OnceWaiter`]

[`OnceWaiter`]: once_waiter::OnceWaiter

This waiter provides functions to do the following:

* Ensure that an initialization function is only run
  once.

* Ensure that an initialization function is only run
  on one thread at a time.

* Run another thread's initialization function in case
  the currently running one panics.

* Wait until the waiter is initialized without providing
  an initialization function.

* Perform the previous bullet points using timeout functions
  which timeout at a certain `Instant` in time, or after
  a certain `Duration`.

## [`Spinlock`]

[`Spinlock`]: spinlock_waiter::Spinlock

A simple spinlock built on atomic primitives.

This is not recommended to be used in most instances, however
can be useful in certain cases.
