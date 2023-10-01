// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::code_block::CodeBlock;
use crate::errors::bail;
use crate::errors::CompilationError;
use crate::errors::Error;
use crate::eval_context::Config;
use crate::eval_context::ContextState;
use crate::runtime::EVCXR_NEXT_RUSTC_WRAPPER;
use anyhow::anyhow;
use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

fn shared_object_prefix() -> &'static str {
    if cfg!(target_os = "macos") {
        "lib"
    } else if cfg!(target_os = "windows") {
        ""
    } else {
        "lib"
    }
}

fn shared_object_extension() -> &'static str {
    if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    }
}

fn shared_object_name_from_crate_name(crate_name: &str) -> String {
    let prefix = shared_object_prefix();
    let extension = shared_object_extension();
    format!("{prefix}{crate_name}.{extension}")
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
    // If the file contents is already correct, then skip writing it again. This
    // is mostly to avoid rewriting Cargo.toml which should change relatively
    // little.
    if fs::read_to_string(&filename)
        .map(|c| c == contents)
        .unwrap_or(false)
    {
        return Ok(());
    }
    if let Err(err) = fs::write(&filename, contents) {
        bail!("Error writing '{:?}': {}", filename, err);
    }
    Ok(())
}

/// On Mac, if we copy the dylib, we get intermittent failures where we end up
/// with the previous version of the file when we shouldn't. On windows, if
/// rename the file, we get errors subsequently when something (perhaps the
/// Windows linker) tries to delete the file that it expects to still be there.
/// On Linux either renaming or copying works, but renaming should be more
/// efficient, so we do that.
#[cfg(windows)]
fn rename_or_copy_so_file(src: &Path, dest: &Path) -> Result<(), Error> {
    // Copy file by reading and writing instead of using std::fs::copy. The src
    // is a hard-linked file and we want to make extra sure that we end up with
    // a completely independent copy.
    fn alt_copy(src: &Path, dest: &Path) -> Result<(), std::io::Error> {
        use std::fs::File;
        std::io::copy(&mut File::open(src)?, &mut File::create(dest)?)?;
        Ok(())
    }
    if let Err(err) = alt_copy(src, dest) {
        bail!("Error copying '{:?}' to '{:?}': {}", src, dest, err);
    }
    Ok(())
}

#[cfg(not(windows))]
fn rename_or_copy_so_file(src: &Path, dest: &Path) -> Result<(), Error> {
    if let Err(err) = fs::rename(src, dest) {
        bail!("Error renaming '{:?}' to '{:?}': {}", src, dest, err);
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct Module {
    build_num: i32,
    last_allow_static: Option<bool>,
    subprocess_path: PathBuf,
}

const CRATE_NAME: &str = "ctx";

impl Module {
    pub(crate) fn new(subprocess_path: PathBuf) -> Result<Module, Error> {
        Ok(Module {
            build_num: 0,
            last_allow_static: None,
            subprocess_path,
        })
    }

    pub(crate) fn so_path(&self, config: &Config) -> PathBuf {
        config
            .deps_dir()
            .join(shared_object_name_from_crate_name(CRATE_NAME))
    }

    // Writes Cargo.toml. Should be called before compile.
    pub(crate) fn write_cargo_toml(&self, state: &ContextState) -> Result<(), Error> {
        write_file(
            state.config.crate_dir(),
            "Cargo.toml",
            &self.get_cargo_toml_contents(state),
        )
    }

    // Writes .cargo/config.toml. Should be called before compile.
    pub(crate) fn write_config_toml(&self, state: &ContextState) -> Result<(), Error> {
        let dot_config_dir = state.config.crate_dir().join(".cargo");
        fs::create_dir_all(dot_config_dir.as_path())?;
        write_file(
            dot_config_dir.as_path(),
            "config.toml",
            &self.get_config_toml_contents(state),
        )
    }

    pub(crate) fn check(
        &mut self,
        code_block: &CodeBlock,
        config: &Config,
    ) -> Result<Vec<CompilationError>, Error> {
        self.write_code(code_block, config)?;
        let output = config
            .cargo_command("check")
            .arg("--message-format=json")
            .output();

        let cargo_output = match output {
            Ok(out) => out,
            Err(err) => bail!("Error running 'cargo check': {}", err),
        };
        let (errors, _non_json_error) = errors_from_cargo_output(&cargo_output, code_block);
        Ok(errors)
    }

    pub(crate) fn compile(
        &mut self,
        code_block: &CodeBlock,
        config: &Config,
    ) -> Result<SoFile, Error> {
        if self.last_allow_static == Some(!config.allow_static_linking) {
            // If allow_static_linking has changed, then we need to rebuild everything.
            config.cargo_command("clean").output()?;
        }
        self.last_allow_static = Some(config.allow_static_linking);
        let mut command = config.cargo_command("rustc");
        if config.time_passes && config.toolchain != "nightly" {
            bail!("time_passes option requires nightly compiler");
        }

        command
            .arg("--target")
            .arg(&config.target)
            .arg("--message-format=json")
            .arg("--")
            .arg("-C")
            .arg("prefer-dynamic")
            .env("CARGO_TARGET_DIR", "target")
            .env("RUSTC", &config.rustc_path);
        if config.linker == "lld" {
            command
                .arg("-C")
                .arg(format!("link-arg=-fuse-ld={}", config.linker));
        }
        if config.allow_static_linking {
            if let Some(sccache) = &config.sccache {
                command.env("RUSTC_WRAPPER", sccache);
            }
        } else {
            command.env("RUSTC_WRAPPER", &self.subprocess_path);
            command.env(
                EVCXR_NEXT_RUSTC_WRAPPER,
                config.sccache.as_deref().unwrap_or(Path::new("")),
            );
        }
        if config.time_passes {
            command.arg("-Ztime-passes");
        }
        self.write_code(code_block, config)?;
        let cargo_output = run_cargo(command, code_block)?;
        if config.time_passes {
            let output = String::from_utf8_lossy(&cargo_output.stderr);
            eprintln!("{output}");
        }
        self.build_num += 1;
        let copied_so_file = config
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
        rename_or_copy_so_file(&self.so_path(config), &copied_so_file)?;
        Ok(SoFile {
            path: copied_so_file,
        })
    }

    fn write_code(&self, code_block: &CodeBlock, config: &Config) -> Result<(), Error> {
        write_file(&config.src_dir(), "lib.rs", &code_block.code_string())?;
        self.maybe_bump_lib_mtime(config);
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn maybe_bump_lib_mtime(&self, _config: &Config) {}

    #[cfg(target_os = "macos")]
    fn maybe_bump_lib_mtime(&self, config: &Config) {
        // Some Macs use a filesystem that only has 1 second precision on file modification
        // timestamps. Cargo uses these timestamps to see if it needs to recompile things, otherwise
        // it just reuses the previous output. We set the modification timestamp on our source file
        // to be 10 seconds in the future. That way it's guaranteed to be newer than any outputs
        // produced by previous runs. In the event that setting the mtime fails, we just ignore it,
        // as this mostly affects tests and we don't want inability to set mtime to break things for
        // users.
        let _ = filetime::set_file_mtime(
            config.src_dir().join("lib.rs"),
            filetime::FileTime::from_unix_time(filetime::FileTime::now().unix_seconds() + 10, 0),
        );
    }

    fn get_cargo_toml_contents(&self, state: &ContextState) -> String {
        let crate_imports = state.format_cargo_deps();
        format!(
            r#"
[package]
name = "{}"
version = "1.0.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]
path = "src/lib.rs"

[profile.dev]
opt-level = {}
debug = false
strip = "debuginfo"
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
            state.opt_level(),
            crate_imports
        )
    }

    // Pass offline mode to cargo through .cargo/config.toml
    fn get_config_toml_contents(&self, state: &ContextState) -> String {
        format!(
            r#"
[net]
offline = {}
"#,
            state.offline_mode()
        )
    }
}

pub(crate) fn wrap_rustc(next_wrapper: &str) {
    match wrap_rustc_helper(next_wrapper) {
        Err(error) => {
            eprintln!("Failed to wrap rustc: {error}");
            std::process::exit(-1);
        }
        Ok(exit_code) => {
            std::process::exit(exit_code);
        }
    }
}

pub(crate) fn wrap_rustc_helper(next_wrapper: &str) -> Result<i32> {
    let num_crate_types = std::env::args_os()
        .filter(|arg| arg == "--crate-type")
        .count();
    let mut args = std::env::args().peekable();
    args.next();
    let rustc = args.next().ok_or_else(|| anyhow!("Insufficient args"))?;
    let mut command;
    if next_wrapper.is_empty() {
        command = std::process::Command::new(rustc);
    } else {
        command = std::process::Command::new(next_wrapper);
        command.arg(rustc);
    }
    let mut got_prefer_dynamic = false;
    while let Some(arg) = args.next() {
        if arg == "-C" {
            let next = args.peek();
            if next.map(|n| n == "prefer-dynamic").unwrap_or_default() {
                got_prefer_dynamic = true;
            }
        }
        command.arg(&arg);

        // If we're compiling as a crate-type of lib and nothing else, then we tell rustc to also
        // compile as a dylib. We still need compile as type lib though, since otherwise cargo
        // recompiles every time - presumably because it detects that the lib file it asked for
        // isn't present.
        if arg == "--crate-type" && num_crate_types == 1 {
            if let Some(crate_type) = args.next() {
                command.arg(&crate_type);
                if crate_type == "lib" {
                    command.arg("--crate-type");
                    command.arg("dylib");
                }
            }
        }
        // Make paths to our dependencies to use dylibs rather than rlibs.
        if arg == "--extern" {
            let ext = args.next().ok_or_else(|| anyhow!("Insufficient args"))?;
            let so_arg = map_extern_arg(&ext);
            command.arg(so_arg);
        }
    }
    if !got_prefer_dynamic {
        command.arg("-C").arg("prefer-dynamic");
    }

    let mut output = command.output()?;

    // If rustc failed and from the error, it looks like it failed due to a missing language
    // feature, then this can often be fixed by enabling the "std" feature on the crate being
    // compiled.
    if did_rustc_fail_due_to_dylib(&output) {
        command.arg("--cfg").arg("feature=\"std\"");
        let alt_output = command.output()?;
        if alt_output.status.success() {
            output = alt_output;
        }
    }

    std::io::stdout().write_all(&output.stdout)?;
    std::io::stderr().write_all(&output.stderr)?;

    Ok(output.status.code().unwrap_or(-1))
}

/// Returns whether rustc emitted an error due to us compiling a dylib.
fn did_rustc_fail_due_to_dylib(output: &std::process::Output) -> bool {
    if output.status.success() {
        return false;
    }
    let Ok(stderr) = std::str::from_utf8(&output.stderr) else {
        return false;
    };
    stderr
        .lines()
        .filter_map(|line| {
            let json_value = json::parse(line).ok()?;
            let message = json_value["message"].as_str()?;
            Some(
                message == "`#[panic_handler]` function required, but not found"
                    || message.starts_with("language item required, but not found:"),
            )
        })
        .any(|b| b)
}

fn map_extern_arg(ext: &str) -> OsString {
    if let Some((crate_name, path)) = ext.split_once('=') {
        let path = Path::new(path);
        let mut so_arg = OsString::from(crate_name);
        so_arg.push("=");
        so_arg.push(path.with_extension(shared_object_extension()));
        return so_arg;
    }
    OsString::from(ext)
}

/// Run a cargo command prepared for the provided `code_block`, processing the
/// command's output.
fn run_cargo(
    mut command: std::process::Command,
    code_block: &CodeBlock,
) -> Result<std::process::Output, Error> {
    use std::io::BufRead;
    use std::io::Read;

    let mb_child = command
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn();
    let mut child = match mb_child {
        Ok(out) => out,
        Err(err) => bail!("Error running 'cargo rustc': {}", err),
    };

    // Collect stdout in a parallel thread
    let mut stdout = child.stdout.take().unwrap();
    let output_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf)?;
        Ok::<_, Error>(buf)
    });

    // Collect stderr synchronously
    let stderr = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut all_errors = Vec::new();
    for mb_line in stderr.split(10) {
        let mut line = mb_line?;
        tee_error_line(&line);
        all_errors.append(&mut line);
        all_errors.push(10);
    }

    let status = child.wait()?;
    let all_output = output_thread.join().expect("Panic in child thread")?;

    let cargo_output = std::process::Output {
        status,
        stdout: all_output,
        stderr: all_errors,
    };
    if cargo_output.status.success() {
        Ok(cargo_output)
    } else {
        let (errors, non_json_error) = errors_from_cargo_output(&cargo_output, code_block);
        if errors.is_empty() {
            if let Some(error) = non_json_error {
                bail!(Error::Message(error));
            } else {
                bail!(Error::Message(format!(
                    "Compilation failed, but no parsable errors were found. STDERR:\n\
                     {}\nSTDOUT:{}\n",
                    String::from_utf8_lossy(&cargo_output.stderr),
                    String::from_utf8_lossy(&cargo_output.stdout)
                )));
            }
        } else {
            bail!(Error::CompilationErrors(errors));
        }
    }
}

/// Process one line from cargo, either copying it to stderr or ignoring.
///
/// At this point it looks for messages about compiling dependency crates.
fn tee_error_line(line: &[u8]) {
    static CRATE_COMPILING: Lazy<regex::bytes::Regex> =
        Lazy::new(|| regex::bytes::Regex::new("^\\s*Compiling (\\w+)(?:\\s+.*)?$").unwrap());
    if let Some(captures) = CRATE_COMPILING.captures(line) {
        let crate_name = captures.get(1).unwrap().as_bytes();
        if crate_name != CRATE_NAME.as_bytes() {
            // write line and the following nl symbol as it was stripped before
            std::io::stderr()
                .write_all(line)
                .expect("Writing to stderr should not fail");
            eprintln!();
        }
    }
}

fn errors_from_cargo_output(
    cargo_output: &std::process::Output,
    code_block: &CodeBlock,
) -> (Vec<CompilationError>, Option<String>) {
    // Our compiler errors should all be in JSON format, but for errors from
    // Cargo errors, we need to add explicit matching for those errors that we
    // expect we might see.
    static KNOWN_NON_JSON_ERRORS: Lazy<Regex> =
        Lazy::new(|| Regex::new("(error: no matching package named)").unwrap());

    let stderr = String::from_utf8_lossy(&cargo_output.stderr);
    let stdout = String::from_utf8_lossy(&cargo_output.stdout);
    let mut non_json_error = None;
    let errors = stderr
        .lines()
        .chain(stdout.lines())
        .filter_map(|line| {
            json::parse(line)
                .ok()
                .and_then(|json| CompilationError::opt_new(json, code_block))
                .or_else(|| {
                    if KNOWN_NON_JSON_ERRORS.is_match(line) {
                        non_json_error = Some(line.to_owned());
                    }
                    None
                })
        })
        .collect();
    (errors, non_json_error)
}

pub(crate) struct SoFile {
    pub(crate) path: PathBuf,
}
