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

#[macro_use]
extern crate lazy_static;

#[cfg(unix)]
#[macro_use]
extern crate sig;

#[macro_use]
mod errors;
mod cargo_metadata;
mod child_process;
mod code_block;
mod command_context;
mod crate_config;
mod eval_context;
#[allow(dead_code)]
mod evcxr_internal_runtime;
mod item;
mod module;
mod runtime;
mod statement_splitter;

pub use crate::command_context::CommandContext;
pub use crate::errors::{CompilationError, Error};
pub use crate::eval_context::{EvalContext, EvalContextOutputs, EvalOutputs};
pub use crate::runtime::runtime_hook;

/// Return the directory that evcxr tools should use for their configuration.
///
/// By default this is the `evcxr` subdirectory of whatever `dirs::config_dir()`
/// returns, but it can be overridden by the `EVCXR_CONFIG_DIR` environment
/// variable.
pub fn config_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("EVCXR_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::config_dir().map(|d| d.join("evcxr")))
}
