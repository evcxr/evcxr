// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use self::artifacts::read_artifacts;
use self::cache::CacheResult;
use crate::code_block::CodeBlock;
use crate::errors::CompilationError;
use crate::errors::Error;
use crate::errors::bail;
use crate::eval_context::Config;
use crate::eval_context::ContextState;
use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use once_cell::sync::Lazy;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

mod artifacts;
pub(crate) mod cache;

pub(crate) const CORE_EXTERN_ENV: &str = "EVCXR_CORE_EXTERN";
pub(crate) const CACHE_ENABLED_ENV: &str = "EVCXR_CACHE_ENABLED";
const EXECUTION_SO_PATH_FILE: &str = ".evcxr-execution-so";
const EXECUTION_SO_PATH_ENV: &str = "EVCXR_EXECUTION_SO_PATH";

pub(crate) fn shared_object_prefix() -> &'static str {
    if cfg!(target_os = "macos") {
        "lib"
    } else if cfg!(target_os = "windows") {
        ""
    } else {
        "lib"
    }
}

pub(crate) fn rlib_prefix() -> &'static str {
    // Rlibs are, fortunately consistent in that they always start with "lib" for all platforms.
    "lib"
}

pub(crate) fn shared_object_extension() -> &'static str {
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
}

const CRATE_NAME: &str = "ctx";

impl Module {
    pub(crate) fn new() -> Result<Module, Error> {
        Ok(Module {
            build_num: 0,
            last_allow_static: None,
        })
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
        let output = config.cargo_command("check").output();

        let cargo_output = match output {
            Ok(out) => out,
            Err(err) => bail!("Error running 'cargo check': {}", err),
        };
        diagnostics_from_cargo_output(&cargo_output, code_block)
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
        let mut command = config.cargo_command("build");
        if config.time_passes && config.toolchain != "nightly" {
            bail!("time_passes option requires nightly compiler");
        }

        self.write_code(code_block, config)?;

        let next_build_num = self.build_num + 1;
        let so_path_file = config.crate_dir().join(EXECUTION_SO_PATH_FILE);
        command.env(EXECUTION_SO_PATH_ENV, &so_path_file);

        match fs::remove_file(&so_path_file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let cargo_output = run_cargo(command, code_block)?;
        if config.time_passes {
            let output = String::from_utf8_lossy(&cargo_output.stderr);
            eprintln!("{output}");
        }

        let so_file = match fs::read_to_string(&so_path_file) {
            Ok(path) => PathBuf::from(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                compiled_so_path(&cargo_output.stdout)?
            }
            Err(error) => return Err(error.into()),
        };

        let copied_so_file = so_file.with_file_name(shared_object_name_from_crate_name(&format!(
            "code_{next_build_num}"
        )));

        rename_or_copy_so_file(&so_file, &copied_so_file)?;
        self.build_num = next_build_num;

        if config.cache_bytes() > 0 {
            crate::module::cache::cleanup(config.cache_bytes())?;
        }

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
edition = "2024"

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

pub(crate) fn wrap_rustc() {
    match wrap_rustc_helper() {
        Err(error) => {
            eprintln!("Failed to wrap rustc: {error}");
            std::process::exit(-1);
        }
        Ok(exit_code) => {
            std::process::exit(exit_code);
        }
    }
}

pub(crate) fn wrap_rustc_helper() -> Result<i32> {
    let mut command = rustc_command()?;

    let cache_result = cache::access_cache(&command)?;
    if matches!(cache_result, CacheResult::Hit) {
        report_execution_so(&command)?;
        return Ok(0);
    }

    let output = command.output()?;

    std::io::stdout().write_all(&output.stdout)?;
    std::io::stderr().write_all(&output.stderr)?;

    if output.status.code() == Some(0) {
        let stderr = std::str::from_utf8(&output.stderr).context("Rustc emitted invalid UTF-8")?;
        let artifacts = read_artifacts(stderr);
        if let CacheResult::Miss(cache_miss) = cache_result {
            cache_miss.update_cache(&artifacts)?;
        }
        report_execution_so(&command)?;
    }

    Ok(output.status.code().unwrap_or(-1))
}

fn report_execution_so(command: &Command) -> Result<()> {
    let Some(source) = execution_so_source(command) else {
        return Ok(());
    };

    let Some(path_file) = std::env::var_os(EXECUTION_SO_PATH_ENV) else {
        return Ok(());
    };
    fs::write(path_file, source.to_string_lossy().as_bytes())?;

    Ok(())
}

fn compiled_so_path(cargo_stdout: &[u8]) -> Result<PathBuf, Error> {
    let expected_name = shared_object_name_from_crate_name(CRATE_NAME);

    for line in String::from_utf8_lossy(cargo_stdout).lines().rev() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        if message["reason"] != "compiler-artifact" || message["target"]["name"] != CRATE_NAME {
            continue;
        }

        let Some(filenames) = message["filenames"].as_array() else {
            continue;
        };

        if let Some(path) = filenames
            .iter()
            .filter_map(|filename| filename.as_str())
            .map(PathBuf::from)
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name == std::ffi::OsStr::new(&expected_name))
            })
        {
            return Ok(path);
        }
    }

    bail!(
        "Cargo did not report the path of the compiled shared object '{}'. Cargo stdout:\n{}",
        expected_name,
        String::from_utf8_lossy(cargo_stdout)
    );
}

fn execution_so_source(command: &Command) -> Option<PathBuf> {
    let mut args = command.get_args();
    let mut is_ctx = false;
    let mut out_dir = None;
    while let Some(arg) = args.next() {
        if arg == "--crate-name" {
            is_ctx = args.next().is_some_and(|name| name == CRATE_NAME);
        } else if arg == "--out-dir" {
            out_dir = args.next().map(PathBuf::from);
        }
    }
    if is_ctx {
        Some(out_dir?.join(shared_object_name_from_crate_name(CRATE_NAME)))
    } else {
        None
    }
}

fn rustc_command() -> Result<Command> {
    let mut args = std::env::args().peekable();
    args.next();
    let rustc = args.next().ok_or_else(|| anyhow!("Insufficient args"))?;
    let mut command = std::process::Command::new(rustc);

    if !should_force_dylibs() {
        command.args(args);
        return Ok(command);
    }

    let num_crate_types = std::env::args_os()
        .filter(|arg| arg == "--crate-type")
        .count();
    let core_extern = std::env::var(CORE_EXTERN_ENV)
        .with_context(|| format!("Internal env var {CORE_EXTERN_ENV}` not set"))?;

    let mut got_prefer_dynamic = false;
    while let Some(arg) = args.next() {
        if arg == "-C" {
            let next = args.peek();
            if next.map(|n| n == "prefer-dynamic").unwrap_or_default() {
                got_prefer_dynamic = true;
            }
        }
        if arg == "-Cprefer-dynamic" {
            got_prefer_dynamic = true;
        }

        // If a static library is being linked into this crate, then we modify the linker arguments
        // to make sure that the whole archive gets linked in, otherwise any symbol not referenced
        // by the functions in this crate (most of them) will be garbage collected by the linker.
        // This would be slightly nicer if we could just pass `-l static:+whole-archive=...` to
        // rustc, however rustc doesn't permit `+whole-archive` and `+bundle` at the same time which
        // is what we want.
        if arg == "-l" {
            let Some(next) = args.next() else { continue };
            if let Some(rest) = next.strip_prefix("static=") {
                command.arg("-Clink-arg=-Wl,--whole-archive");
                command.arg("-l").arg(rest);
                command.arg("-Clink-arg=-Wl,--no-whole-archive");
            } else {
                command.arg(arg);
                command.arg(next);
            }
            continue;
        }

        command.arg(&arg);

        // If we're compiling as a crate-type of lib and nothing else, then we tell rustc to also
        // compile as a dylib. We still need compile as type lib though, since otherwise cargo
        // recompiles every time - presumably because it detects that the lib file it asked for
        // isn't present.
        if arg == "--crate-type"
            && num_crate_types == 1
            && let Some(crate_type) = args.next()
        {
            command.arg(&crate_type);
            if crate_type == "lib" {
                command.arg("--crate-type");
                command.arg("dylib");
            }
        }
        // Make paths to our dependencies to use dylibs rather than rlibs.
        if arg == "--extern" {
            let ext = args.next().ok_or_else(|| anyhow!("Insufficient args"))?;
            let (so_arg, rmeta_arg) = map_extern_arg(&ext);
            command.arg(so_arg);
            if let Some(rmeta_arg) = rmeta_arg {
                command.arg("--extern").arg(rmeta_arg);
            }
        }
    }
    command.arg("--extern").arg(core_extern);
    if !got_prefer_dynamic {
        command.arg("-C").arg("prefer-dynamic");
    }
    Ok(command)
}

fn should_force_dylibs() -> bool {
    std::env::var(crate::runtime::FORCE_DYLIB_ENV).is_ok()
}

fn map_extern_arg(ext: &str) -> (OsString, Option<OsString>) {
    if let Some((crate_name, path)) = ext.split_once('=') {
        let mut path = PathBuf::from(path);
        let mut so_arg = OsString::from(crate_name);
        // Remove the rlib prefix and add the shared object prefix (if any). On unix platforms this
        // is redundant because both are "lib", but it's necessary on Windows. We do it
        // unconditionally though because it's cheap and it makes sure the code always gets tested.
        if let Some(without_prefix) = path
            .file_name()
            .and_then(|f| f.to_str())
            .and_then(|f| f.strip_prefix(rlib_prefix()))
        {
            let prefix = shared_object_prefix();
            path = path.with_file_name(format!("{prefix}{without_prefix}"));
        }
        so_arg.push("=");
        let path = path.with_extension(shared_object_extension());
        // The shared object might not exist yet if we're doing a `cargo check`, so we only attempt
        // to use it if it actually exists.
        if path.exists() {
            let rmeta_path = path.with_extension("rmeta");
            so_arg.push(path);
            // Handle embed-metadata=no
            let rmeta_arg = rmeta_path.exists().then(|| {
                let mut arg = OsString::from(crate_name);
                arg.push("=");
                arg.push(rmeta_path);
                arg
            });
            return (so_arg, rmeta_arg);
        }
    }
    (OsString::from(ext), None)
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
        let errors = diagnostics_from_cargo_output(&cargo_output, code_block)?
            .into_iter()
            .filter(|diagnostic| diagnostic.level() == "error")
            .collect();
        bail!(Error::CompilationErrors(errors));
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

// Keep warnings for callers of `check`, but don't let them hide a Cargo failure
// that has no compiler error diagnostics.
fn diagnostics_from_cargo_output(
    cargo_output: &std::process::Output,
    code_block: &CodeBlock,
) -> Result<Vec<CompilationError>, Error> {
    let stderr = String::from_utf8_lossy(&cargo_output.stderr);
    let stdout = String::from_utf8_lossy(&cargo_output.stdout);
    let diagnostics: Vec<CompilationError> = stderr
        .lines()
        .chain(stdout.lines())
        .filter_map(|line| {
            serde_json::from_str(line)
                .ok()
                .and_then(|json| CompilationError::opt_new(json, code_block))
        })
        .collect();
    if !cargo_output.status.success()
        && !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level() == "error")
    {
        bail!(Error::Message(format!(
            "Compilation failed, but no error-level compiler diagnostics were found. STDERR:\n\
             {stderr}\nSTDOUT:{stdout}\n"
        )));
    }
    Ok(diagnostics)
}

pub(crate) struct SoFile {
    pub(crate) path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::process::ExitStatus;
    use std::process::Output;

    fn cargo_output(success: bool, diagnostics: &[serde_json::Value], stderr: &str) -> Output {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;

        Output {
            status: ExitStatus::from_raw(if success { 0 } else { 1 << 8 }),
            stdout: diagnostics
                .iter()
                .map(|diagnostic| format!("{diagnostic}\n"))
                .collect::<String>()
                .into_bytes(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn diagnostic(level: &str) -> serde_json::Value {
        json!({
            "reason": "compiler-message",
            "message": {
                "level": level,
                "message": format!("a compiler {level}"),
                "spans": [],
                "children": [],
            },
        })
    }

    #[test]
    fn cargo_failure_preserves_output_without_compiler_errors() {
        for diagnostics in [vec![], vec![diagnostic("warning")]] {
            for stderr in [
                "error while loading shared libraries: libmissing.so",
                "error: no matching package named `missing` found\nlocation searched: registry",
            ] {
                let output = cargo_output(false, &diagnostics, stderr);
                let error = diagnostics_from_cargo_output(&output, &CodeBlock::new()).unwrap_err();
                let Error::Message(message) = error else {
                    panic!("Expected Cargo failure, got {error:?}");
                };
                assert!(message.contains(stderr), "{message}");
                assert!(message.contains(std::str::from_utf8(&output.stdout).unwrap()));
            }
        }
    }

    #[test]
    fn check_diagnostics_preserve_warnings() {
        for success in [false, true] {
            let mut diagnostics = vec![diagnostic("warning")];
            if !success {
                diagnostics.push(diagnostic("error"));
            }
            let output = cargo_output(success, &diagnostics, "");
            let parsed = diagnostics_from_cargo_output(&output, &CodeBlock::new()).unwrap();
            assert_eq!(parsed.len(), diagnostics.len());
            assert_eq!(parsed[0].level(), "warning");
            if !success {
                assert_eq!(parsed[1].level(), "error");
            }
        }
    }

    #[test]
    fn child_note_retains_parent_severity() {
        let code = CodeBlock::new().other_user_code("fn user_code() {}".to_owned());
        for level in ["error", "warning"] {
            let mut parent = diagnostic(level);
            parent["message"]["children"] = json!([{
                "level": "note",
                "message": "required by this user code",
                "spans": [{
                    "file_name": "src/lib.rs",
                    "line_start": 1,
                    "line_end": 1,
                    "column_start": 1,
                    "column_end": 2,
                }],
            }]);
            let output = cargo_output(level != "error", &[parent], "");
            let parsed = diagnostics_from_cargo_output(&output, &code).unwrap();
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].level(), level);
            assert_eq!(parsed[0].message(), "required by this user code");
            assert!(parsed[0].is_from_user_code());
        }
    }

    #[cfg(unix)]
    fn run_fake_cargo(success: bool, diagnostics: &[serde_json::Value]) -> Result<Output, Error> {
        let output = cargo_output(success, diagnostics, "missing shared library");
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("printf '%s' \"$1\"; printf '%s' \"$2\" >&2; exit \"$3\"")
            .arg("fake-cargo")
            .arg(String::from_utf8(output.stdout).unwrap())
            .arg(String::from_utf8(output.stderr).unwrap())
            .arg(if success { "0" } else { "1" });
        run_cargo(command, &CodeBlock::new())
    }

    #[test]
    #[cfg(unix)]
    fn failed_build_with_warnings_reports_cargo_failure() {
        let error = run_fake_cargo(false, &[diagnostic("warning")]).unwrap_err();
        let Error::Message(message) = error else {
            panic!("Expected Cargo failure, got {error:?}");
        };
        assert!(message.contains("missing shared library"), "{message}");
    }

    #[test]
    #[cfg(unix)]
    fn failed_build_returns_only_compiler_errors() {
        let error =
            run_fake_cargo(false, &[diagnostic("warning"), diagnostic("error")]).unwrap_err();
        let Error::CompilationErrors(errors) = error else {
            panic!("Expected compiler errors, got {error:?}");
        };
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].level(), "error");
    }

    #[test]
    #[cfg(unix)]
    fn successful_build_with_warnings_succeeds() {
        assert!(run_fake_cargo(true, &[diagnostic("warning")]).is_ok());
    }
}
