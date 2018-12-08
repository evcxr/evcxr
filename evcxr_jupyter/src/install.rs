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

use dirs;
use failure::Error;
use std::io::Write;
use std::path::PathBuf;
use std::{env, fs};

const LOGO_32X32: &[u8] = include_bytes!("../third_party/rust/rust-logo-32x32.png");
const LOGO_64X64: &[u8] = include_bytes!("../third_party/rust/rust-logo-64x64.png");
const LOGO_LICENSE: &[u8] = include_bytes!("../third_party/rust/LICENSE.md");

pub(crate) fn install() -> Result<(), Error> {
    let kernel_dir = get_kernel_dir()?;
    fs::create_dir_all(&kernel_dir)?;
    let current_exe_path = env::current_exe()?;
    let current_exe = current_exe_path
        .to_str()
        .ok_or_else(|| format_err!("current exe path isn't valid UTF-8"))?;
    let kernel_json = object! {
        "argv" => array![current_exe, "--control_file", "{connection_file}"],
        "display_name" => "Rust",
        "language" => "rust",
        "interrupt_mode" => "message",
    };
    let kernel_json_filename = kernel_dir.join("kernel.json");
    println!("Writing {}", kernel_json_filename.to_string_lossy());
    kernel_json.write_pretty(&mut fs::File::create(kernel_json_filename)?, 2)?;
    install_resource(&kernel_dir, "logo-32x32.png", LOGO_32X32)?;
    install_resource(&kernel_dir, "logo-64x64.png", LOGO_64X64)?;
    install_resource(&kernel_dir, "logo-LICENSE.md", LOGO_LICENSE)?;
    println!("Installation complete");
    Ok(())
}

pub(crate) fn install_resource(
    dir: &PathBuf,
    filename: &str,
    bytes: &'static [u8],
) -> Result<(), Error> {
    let res_path = dir.join(filename);
    println!("Writing {}", res_path.to_string_lossy());
    let mut file = fs::File::create(res_path)?;
    file.write_all(bytes)?;
    Ok(())
}

pub(crate) fn uninstall() -> Result<(), Error> {
    let kernel_dir = get_kernel_dir()?;
    println!("Deleting {}", kernel_dir.to_string_lossy());
    fs::remove_dir_all(kernel_dir)?;
    println!("Uninstall complete");
    Ok(())
}

// https://jupyter-client.readthedocs.io/en/latest/kernels.html
fn get_kernel_dir() -> Result<PathBuf, Error> {
    let jupyter_dir = if let Ok(dir) = env::var("JUPYTER_CONFIG_DIR") {
        PathBuf::from(dir)
    } else if let Ok(dir) = env::var("JUPYTER_PATH") {
        PathBuf::from(dir)
    } else if let Some(dir) = get_user_kernel_dir() {
        dir
    } else {
        bail!("Couldn't get XDG data directory");
    };
    Ok(jupyter_dir.join("kernels").join("rust"))
}

#[cfg(not(target_os = "macos"))]
fn get_user_kernel_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|data_dir| data_dir.join("jupyter"))
}

#[cfg(target_os = "macos")]
fn get_user_kernel_dir() -> Option<PathBuf> {
    dirs::data_dir().and_then(|d| d.parent().map(|data_dir| data_dir.join("Jupyter")))
}
