// Copyright 2018 Google Inc.
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

// This file is both a module of evcxr and is included via include_str! then
// built as a crate itself. The latter is the primary use-case. It's included as
// a submodule only so that constants can be shared.

use std::any::Any;
use std::collections::HashMap;
use std::{io, process};

pub const PANIC_NOTIFICATION: &str = "EVCXR_PANIC_NOTIFICATION";
pub const VARIABLE_CHANGED_TYPE: &str = "EVCXR_VARIABLE_CHANGED_TYPE:";

pub struct VariableStore {
    variables: HashMap<String, Box<dyn Any + 'static>>,
}

impl VariableStore {
    pub fn new() -> VariableStore {
        VariableStore {
            variables: HashMap::new(),
        }
    }

    pub fn assert_copy_type<T: Copy>(&self, _: T) {}

    pub fn put_variable<T: 'static>(&mut self, name: &str, value: T) {
        self.variables.insert(name.to_owned(), Box::new(value));
    }

    pub fn check_variable<T: 'static>(&mut self, name: &str) -> bool {
        if let Some(v) = self.variables.get(name) {
            if v.downcast_ref::<T>().is_none() {
                eprintln!(
                    "The type of the variable {} was redefined, so was lost.",
                    name
                );
                println!("{}{}", VARIABLE_CHANGED_TYPE, name);
                return false;
            }
        }
        true
    }

    pub fn take_variable<T: 'static>(&mut self, name: &str) -> T {
        match self.variables.remove(name) {
            Some(v) => {
                if let Ok(value) = v.downcast() {
                    *value
                } else {
                    // Shouldn't happen so long as check_variable was called.
                    panic!("Variable changed type");
                }
            }
            None => panic!("Variable '{}' has gone missing", name),
        }
    }

    pub fn merge(&mut self, mut other: VariableStore) {
        self.variables.extend(other.variables.drain());
    }
}

pub fn create_variable_store() -> *mut VariableStore {
    Box::into_raw(Box::new(VariableStore::new()))
}

pub fn send_text_plain(text: &str) {
    use std::io::Write;
    fn try_send_text(text: &str) -> io::Result<()> {
        let stdout = io::stdout();
        let mut output = stdout.lock();
        output.write_all(b"EVCXR_BEGIN_CONTENT text/plain\n")?;
        output.write_all(text.as_bytes())?;
        output.write_all(b"\nEVCXR_END_CONTENT\n")?;
        Ok(())
    }
    if let Err(error) = try_send_text(text) {
        eprintln!("Failed to send content to parent: {:?}", error);
        process::exit(1);
    }
}

pub fn notify_panic() {
    println!("{}", PANIC_NOTIFICATION);
}
