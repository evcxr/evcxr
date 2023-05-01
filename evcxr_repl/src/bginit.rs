// Copyright 2021 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use parking_lot::MappedMutexGuard;
use parking_lot::Mutex;
use parking_lot::Once;
use std::thread::JoinHandle;

// We're using `parking_lot`'s mutex so that we can get the mapped guards — it's
// already in our dependency tree, and this lets us hide the fact that the
// actual mutex is a `Mutex<BgInitState<T>>`, and not a `Mutex<T>`.
pub type BgInitMutexGuard<'a, T> = MappedMutexGuard<'a, T>;

/// A mutex which also initializes the locked value asynchronously, on a
/// background thread. When `lock()` is called for the first time, we join the
/// background thread, blocking until the value is available.
///
/// Admittedly, it's a bit odd to combine the concepts of asynchronous
/// initialization and mutual exclusion. In an ideal world this would probably
/// be `BgInit<T>`, and then you could compose it with a mutex like
/// `BgInit<Mutex<T>>`. However, combining them simplifies the implementation;
/// we need a mutex internally to implement this without `unsafe`, and the code
/// that needs this type needs the mutex.
pub struct BgInitMutex<T> {
    // guards us `join()`ing the background thread, and transitioning from
    // `BgInitState::Pending` to `BgInitState::Ready`.
    init: Once,
    // Call `ensure_ready()` before accessing.
    state: Mutex<BgInitState<T>>,
}

impl<T> BgInitMutex<T> {
    pub fn new<F>(f: F) -> Self
    where
        F: Send + 'static + FnOnce() -> T,
        T: Send + 'static,
    {
        Self {
            state: Mutex::new(BgInitState::Pending(std::thread::spawn(f))),
            init: Once::new(),
        }
    }
    fn ensure_ready(&self) {
        self.init.call_once(|| {
            // Use try_lock to detect misuse in a way other than deadlocking /
            // having race conditions / other undesirable things...
            let mut state_guard = self
                .state
                .try_lock()
                .expect("bug: nobody should be allowed to lock `self.state` yet");
            // Swap out the state, leaving Failed in its place. This is required
            // because we need to move the join handle out of the state in order
            // to join it.
            let state = core::mem::replace(&mut *state_guard, BgInitState::Failed);
            match state {
                BgInitState::Pending(jh) => {
                    let value = jh.join().unwrap_or_else(|e| {
                        // This will both poison the `Once`, and leave the state
                        // as Failed permanently (both of these are fine).
                        //
                        // We use `resume_unwind` here rather than `unwrap()` to
                        // avoid double-printing the panic info, and printing a
                        // non-useful backtrace.
                        std::panic::resume_unwind(e);
                    });
                    *state_guard = BgInitState::Ready(value);
                }
                st => wrong_state(&st, "Pending"),
            }
        });
    }

    pub fn lock(&self) -> MappedMutexGuard<'_, T> {
        self.ensure_ready();
        let state = self.state.lock();

        parking_lot::MutexGuard::map(state, |state| match state {
            BgInitState::Ready(v) => v,
            st => wrong_state(st, "Ready"),
        })
    }
}

// The inititalization state. The states here more-or-less directly correspond
// to different states of the `Once` inside the parent.
//
// - when `BgInitMutex::init` is in `OnceState::New`, the state should be
//   `BgInitState::Pending`
// - when `BgInitMutex::init` is in `OnceState::Done`, the state should be
//   `BgInitState::Ready`
// - when `BgInitMutex::init` is in `OnceState::Poisoned`, the state should be
//   `BgInitState::Failed`
//
// There's no direct equivalent to `OnceState::InProgress`, but most of it is
// spent as `Failed`.
enum BgInitState<T> {
    Ready(T),
    Pending(JoinHandle<T>),
    // Transitioning from `Pending` to `Ready` requires moving the `JoinHandle`
    // out of `Pending`. We can't do that without leaving something in it's
    // place, and so we leave a `Failed`.
    Failed,
}

#[track_caller]
#[cold]
fn wrong_state<T>(st: &BgInitState<T>, wanted: &str) -> ! {
    let actual = match st {
        BgInitState::Ready(_) => "Ready",
        BgInitState::Pending(_) => "Pending",
        BgInitState::Failed => "Failed",
    };
    panic!("bug: BgInitState should be {wanted:?}, but is {actual:?}",);
}
