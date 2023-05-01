// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

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
