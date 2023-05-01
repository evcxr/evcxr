// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use anyhow::anyhow;
use anyhow::bail;
use anyhow::Result;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

const LOGO_32X32: &[u8] = include_bytes!("../third_party/rust/rust-logo-32x32.png");
const LOGO_64X64: &[u8] = include_bytes!("../third_party/rust/rust-logo-64x64.png");
const LOGO_LICENSE: &[u8] = include_bytes!("../third_party/rust/LICENSE.md");
const KERNEL_JS: &[u8] = include_bytes!("../client/kernel.js");
const LINT_JS: &[u8] = include_bytes!("../third_party/CodeMirror/addons/lint/lint.js");
const LINT_CSS: &[u8] = include_bytes!("../third_party/CodeMirror/addons/lint/lint.css");
const LINT_LICENSE: &[u8] = include_bytes!("../third_party/CodeMirror/LICENSE");
const VERSION_TXT: &[u8] = include_bytes!("../client/version.txt");

pub(crate) fn install() -> Result<()> {
    let kernel_dir = get_kernel_dir()?;
    fs::create_dir_all(&kernel_dir)?;
    let current_exe_path = env::current_exe()?;
    let current_exe = current_exe_path
        .to_str()
        .ok_or_else(|| anyhow!("current exe path isn't valid UTF-8"))?;
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
    install_resource(&kernel_dir, "kernel.js", KERNEL_JS)?;
    install_resource(&kernel_dir, "lint.js", LINT_JS)?;
    install_resource(&kernel_dir, "lint.css", LINT_CSS)?;
    install_resource(&kernel_dir, "lint-LICENSE", LINT_LICENSE)?;
    install_resource(&kernel_dir, "version.txt", VERSION_TXT)?;
    println!("Installation complete");
    Ok(())
}

/// Checks if the current installation is out-of-date, by looking at what's in
/// version.txt. If it is out of date, then updates it.
pub(crate) fn update_if_necessary() -> Result<()> {
    let kernel_dir = get_kernel_dir()?;
    // If the kernel directory doesn't exist, then we're probably being run from
    // a wrapper, so we shouldn't "update", since that would in effect be
    // installing ourselves when we weren't already installed.
    if !kernel_dir.exists() {
        return Ok(());
    }
    let installed_version = std::fs::read(kernel_dir.join("version.txt")).unwrap_or_default();
    if installed_version != VERSION_TXT {
        install()?;
        eprintln!(
            "\n\n==================================================================\n\
            Updated Evcxr Jupyter installation. Note, updates unfortunately \n\
            won't take effect until the next time you start jupyter notebook.\n\
            ==================================================================\n"
        );
    }
    Ok(())
}

pub(crate) fn install_resource(dir: &Path, filename: &str, bytes: &'static [u8]) -> Result<()> {
    let res_path = dir.join(filename);
    println!("Writing {}", res_path.to_string_lossy());
    let mut file = fs::File::create(res_path)?;
    file.write_all(bytes)?;
    Ok(())
}

pub(crate) fn uninstall() -> Result<()> {
    let kernel_dir = get_kernel_dir()?;
    println!("Deleting {}", kernel_dir.to_string_lossy());
    fs::remove_dir_all(kernel_dir)?;
    println!("Uninstall complete");
    Ok(())
}

// https://jupyter-client.readthedocs.io/en/latest/kernels.html
fn get_kernel_dir() -> Result<PathBuf> {
    let jupyter_dir = if let Ok(dir) = env::var("JUPYTER_PATH") {
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
