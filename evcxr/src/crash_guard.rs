// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

pub(crate) struct CrashGuard<F: Fn()> {
    armed: bool,
    callback: F,
}

impl<F: Fn()> CrashGuard<F> {
    pub(crate) fn new(callback: F) -> CrashGuard<F> {
        CrashGuard {
            armed: true,
            callback,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<F: Fn()> Drop for CrashGuard<F> {
    fn drop(&mut self) {
        if self.armed {
            (self.callback)();
        }
    }
}
