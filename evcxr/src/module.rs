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

use code_block::CodeBlock;
use errors::{CompilationError, Error};
use json;
use regex::Regex;
use std;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use EvalContext;

fn shared_object_name_from_crate_name(crate_name: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{}.dylib", crate_name)
    } else if cfg!(target_os = "windows") {
        format!("{}.dll", crate_name)
    } else {
        format!("lib{}.so", crate_name)
    }
}

pub(crate) struct Module {
    pub(crate) crate_name: String,
    pub(crate) crate_dir: PathBuf,
    pub(crate) user_fn_name: String,
    target_dir: PathBuf,
    pub(crate) so_path: PathBuf,
    rs_filename: PathBuf,
    manual_deps: Vec<String>,
}

impl Module {
    pub(crate) fn new(
        eval_context: &EvalContext,
        crate_name: &str,
        previous_module: Option<&Module>,
    ) -> Result<Module, Error> {
        let target_dir = eval_context.target_dir();
        let crate_dir = eval_context.tmpdir_path.join(&crate_name);
        let src_dir = crate_dir.join("src");
        fs::create_dir_all(&src_dir)?;
        let rs_filename = src_dir.join("lib.rs");
        let so_path = eval_context
            .deps_dir()
            .join(shared_object_name_from_crate_name(crate_name));

        let module = Module {
            so_path,
            crate_name: crate_name.to_owned(),
            user_fn_name: format!("run_{}", crate_name),
            crate_dir,
            target_dir,
            rs_filename,
            manual_deps: vec![],
        };
        if let Some(previous_module) = previous_module {
            // Copy the lock file from our previous compilation, if any, to
            // avoid having Cargo recreate it, which would be time consuming
            // (more than a second on my machine).
            fs::copy(previous_module.cargo_lock_path(), &module.cargo_lock_path())?;
        }
        Ok(module)
    }

    fn cargo_lock_path(&self) -> PathBuf {
        self.crate_dir.join("Cargo.lock")
    }

    pub(crate) fn write_sources_and_compile(
        &mut self,
        eval_context: &EvalContext,
        code_block: &CodeBlock,
    ) -> Result<(), Error> {
        self.write_cargo_toml(eval_context)?;
        self.compile(code_block)
    }

    // Writes Cargo.toml. Should be called before compile (or just use
    // write_sources_and_compile).
    pub(crate) fn write_cargo_toml(&mut self, eval_context: &EvalContext) -> Result<(), Error> {
        fs::write(
            self.crate_dir.join("Cargo.toml"),
            self.get_cargo_toml_contents(eval_context),
        )?;
        Ok(())
    }

    pub(crate) fn add_dep(&mut self, module: &Module) {
        self.manual_deps.push(module.crate_name.clone());
    }

    pub(crate) fn compile(&mut self, code_block: &CodeBlock) -> Result<(), Error> {
        fs::write(self.rs_filename.clone(), code_block.to_string().as_bytes())?;

        // Our compiler errors should all be in JSON format, but for errors from Cargo errors, we
        // need to add explicit matching for those errors that we expect we might see.
        lazy_static! {
            static ref KNOWN_NON_JSON_ERRORS: Regex =
                Regex::new("(error: no matching package named)").unwrap();
        }

        let mut command = std::process::Command::new("cargo");
        command
            .env("CARGO_TARGET_DIR", &self.target_dir)
            .arg("rustc")
            .arg("--")
            .arg("-C")
            .arg("prefer-dynamic")
            .arg("--error-format")
            .arg("json")
            .current_dir(&self.crate_dir);
        let cargo_output = command.output()?;
        if !cargo_output.status.success() {
            let stderr = String::from_utf8_lossy(&cargo_output.stderr);
            let mut non_json_error = None;
            let errors: Vec<CompilationError> = stderr
                .lines()
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
                    bail!(Error::JustMessage(error.to_owned()));
                } else {
                    bail!(Error::JustMessage(format!(
                        "Compilation failed, but no parsable errors were found. STDERR:\n\
                         {}\nSTDOUT:{}\n",
                        stderr,
                        String::from_utf8_lossy(&cargo_output.stdout)
                    )));
                }
            } else {
                bail!(Error::CompilationErrors(errors));
            }
        }
        Ok(())
    }

    fn get_cargo_toml_contents(&self, eval_context: &EvalContext) -> String {
        use std::fmt::Write;
        let mut loaded_module_deps = String::new();
        let combined_deps = eval_context
            .modules_iter()
            .map(|m| &m.crate_name)
            .chain(self.manual_deps.iter())
            .collect::<HashSet<_>>();
        for crate_name in combined_deps {
            writeln!(
                &mut loaded_module_deps,
                "{} = {{ path = \"../{}\" }}",
                crate_name, crate_name
            )
            .unwrap();
        }
        let crate_imports = eval_context.format_cargo_deps();
        format!(
            r#"
[package]
name = "{}"
version = "1.0.0"

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
{}
"#,
            self.crate_name,
            eval_context.opt_level(),
            loaded_module_deps,
            crate_imports
        )
    }
}
