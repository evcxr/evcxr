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

// This file is both a module of evcxr and is included via include_str! then
// built as a crate itself. The latter is the primary use-case. It's included as
// a submodule only so that constants can be shared.

pub const VARIABLE_CHANGED_TYPE: &str = "EVCXR_VARIABLE_CHANGED_TYPE:";
pub const USER_ERROR_OCCURRED: &str = "EVCXR_ERROR_OCCURRED";

pub struct VariableStore {
    variables: std::collections::HashMap<String, Box<dyn std::any::Any + 'static>>,
}

impl VariableStore {
    pub fn new() -> VariableStore {
        VariableStore {
            variables: std::collections::HashMap::new(),
        }
    }

    pub fn put_variable<T: 'static>(&mut self, name: &str, value: T) {
        self.variables.insert(name.to_owned(), Box::new(value));
    }

    pub fn check_variable<T: 'static>(&mut self, name: &str) -> bool {
        if let Some(v) = self.variables.get(name) {
            if v.downcast_ref::<T>().is_none() {
                eprintln!("The type of the variable {name} was redefined, so was lost.",);
                println!("{VARIABLE_CHANGED_TYPE}{name}");
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
            None => panic!("Variable '{name}' has gone missing"),
        }
    }

    pub fn lazy_arc<T: 'static, F: FnOnce() -> T>(
        &mut self,
        name: &str,
        create: F,
    ) -> std::sync::Arc<T> {
        if let Some(value) = self
            .variables
            .entry(name.to_owned())
            .or_insert_with(|| Box::new(std::sync::Arc::new(create())))
            .downcast_mut()
        {
            std::sync::Arc::clone(value)
        } else {
            panic!("lazy_arc {name} changed type");
        }
    }

    pub fn merge(&mut self, mut other: VariableStore) {
        self.variables.extend(other.variables.drain());
    }
}

pub fn create_variable_store() -> *mut VariableStore {
    Box::into_raw(Box::new(VariableStore::new()))
}
