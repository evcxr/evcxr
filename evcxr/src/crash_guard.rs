// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

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
