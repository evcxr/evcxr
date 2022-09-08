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

#[macro_use]
extern crate json;

use anyhow::anyhow;
use anyhow::bail;
use anyhow::Result;

mod connection;
mod control_file;
mod core;
mod install;
mod jupyter_message;

fn run(control_file_name: &str) -> Result<()> {
    let config = control_file::Control::parse_file(control_file_name)?;
    core::Server::run(&config)
}

fn main() -> Result<()> {
    evcxr::runtime_hook();
    let mut args = std::env::args();
    let bin = args.next().unwrap();
    if let Some(arg) = args.next() {
        match arg.as_str() {
            "--control_file" => {
                if let Err(error) = install::update_if_necessary() {
                    eprintln!("Warning: tried to update client, but failed: {}", error);
                }
                return run(&args.next().ok_or_else(|| anyhow!("Missing control file"))?);
            }
            "--install" => return install::install(),
            "--uninstall" => return install::uninstall(),
            "--help" => {}
            x => bail!("Unrecognised option {}", x),
        }
    }
    println!("To install, run:\n  {} --install", bin);
    println!("To uninstall, run:\n  {} --uninstall", bin);
    Ok(())
}

#[cfg(feature = "mimalloc")]
#[global_allocator]
static MIMALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;
