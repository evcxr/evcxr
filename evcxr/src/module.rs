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

use crate::code_block::CodeBlock;
use crate::errors::{CompilationError, Error};
use json;
use regex::Regex;
use std;
use std::fs;
use std::path::{Path, PathBuf};
use crate::EvalContext;

fn shared_object_name_from_crate_name(crate_name: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{}.dylib", crate_name)
    } else if cfg!(target_os = "windows") {
        format!("{}.dll", crate_name)
    } else {
        format!("lib{}.so", crate_name)
    }
}

fn create_dir(dir: &Path) -> Result<(), Error> {
    if let Err(err) = fs::create_dir_all(dir) {
        bail!("Error creating directory '{:?}': {}", dir, err);
    }
    Ok(())
}

fn write_file(dir: &Path, basename: &str, contents: &str) -> Result<(), Error> {
    create_dir(dir)?;
    let filename = dir.join(basename);
    if let Err(err) = fs::write(&filename, contents) {
        bail!("Error writing '{:?}': {}", filename, err);
    }
    Ok(())
}

pub(crate) struct Module {
    pub(crate) tmpdir: PathBuf,
    build_num: i32,
}

const CRATE_NAME: &str = "ctx";

impl Module {
    pub(crate) fn new(tmpdir: PathBuf) -> Result<Module, Error> {
        let module = Module {
            tmpdir,
            build_num: 0,
        };
        Ok(module)
    }

    pub(crate) fn deps_dir(&self) -> PathBuf {
        self.target_dir().join("debug").join("deps")
    }

    fn target_dir(&self) -> PathBuf {
        self.tmpdir.join("target")
    }

    fn so_path(&self) -> PathBuf {
        self.deps_dir()
            .join(shared_object_name_from_crate_name(CRATE_NAME))
    }

    fn src_dir(&self) -> PathBuf {
        self.tmpdir.join("src")
    }

    pub(crate) fn crate_dir(&self) -> &Path {
        &self.tmpdir
    }

    // Writes Cargo.toml. Should be called before compile.
    pub(crate) fn write_cargo_toml(&self, eval_context: &EvalContext) -> Result<(), Error> {
        write_file(
            self.crate_dir(),
            "Cargo.toml",
            &self.get_cargo_toml_contents(eval_context),
        )
    }

    pub(crate) fn compile(&mut self, code_block: &CodeBlock) -> Result<SoFile, Error> {
        write_file(&self.src_dir(), "lib.rs", &code_block.to_string())?;

        // Our compiler errors should all be in JSON format, but for errors from Cargo errors, we
        // need to add explicit matching for those errors that we expect we might see.
        lazy_static! {
            static ref KNOWN_NON_JSON_ERRORS: Regex =
                Regex::new("(error: no matching package named)").unwrap();
        }

        let mut command = std::process::Command::new("cargo");
        command
            .env("CARGO_TARGET_DIR", &self.target_dir())
            .arg("rustc")
            .arg("--message-format=json")
            .arg("--")
            .arg("-C")
            .arg("prefer-dynamic")
            .current_dir(self.crate_dir());
        let cargo_output = command.output()?;
        if !cargo_output.status.success() {
            let stderr = String::from_utf8_lossy(&cargo_output.stderr);
            let stdout = String::from_utf8_lossy(&cargo_output.stdout);
            let mut non_json_error = None;
            let errors: Vec<CompilationError> = stderr
                .lines()
                .chain(stdout.lines())
                .filter_map(|line| {
                    json::parse(&line)
                        .ok()
                        .and_then(|json| CompilationError::opt_new(json, code_block))
                        .or_else(|| {
                            if KNOWN_NON_JSON_ERRORS.is_match(line) {
                                non_json_error = Some(line);
                            }
                            None
                        })
                })
                .collect();
            if errors.is_empty() {
                if let Some(error) = non_json_error {
                    bail!(Error::Message(error.to_owned()));
                } else {
                    bail!(Error::Message(format!(
                        "Compilation failed, but no parsable errors were found. STDERR:\n\
                         {}\nSTDOUT:{}\n",
                        stderr, stdout
                    )));
                }
            } else {
                bail!(Error::CompilationErrors(errors));
            }
        }
        self.build_num += 1;
        let copied_so_file = self
            .deps_dir()
            .join(shared_object_name_from_crate_name(&format!(
                "code_{}",
                self.build_num
            )));
        // Every time we compile, the output file is the same. We need to
        // renamed it so that we have a unique filename, otherwise we wouldn't
        // be able to load the result of the next compilation. Also, on Windows,
        // a loaded dll gets locked, so we couldn't even compile a second time
        // if we didn't load a different file.
        if let Err(err) = fs::rename(self.so_path(), &copied_so_file) {
            bail!(
                "Error copying '{:?}' to '{:?}': {}",
                self.so_path(),
                copied_so_file,
                err
            );
        }
        Ok(SoFile {
            path: copied_so_file,
        })
    }

    fn get_cargo_toml_contents(&self, eval_context: &EvalContext) -> String {
        let crate_imports = eval_context.format_cargo_deps();
        format!(
            r#"
[package]
name = "{}"
version = "1.0.0"
edition = "2018"

[lib]
crate-type = ["dylib"]

[profile.dev]
opt-level = {}
debug = true
rpath = true
lto = false
debug-assertions = true
codegen-units = 16
panic = 'unwind'
incremental = true
overflow-checks = true

[dependencies]
{}
"#,
            CRATE_NAME,
            eval_context.opt_level(),
            crate_imports
        )
    }
}

pub(crate) struct SoFile {
    pub(crate) path: PathBuf,
}
