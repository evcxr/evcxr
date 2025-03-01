// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use self::artifacts::read_artifacts;
use self::cache::CacheResult;
use crate::code_block::CodeBlock;
use crate::errors::bail;
use crate::errors::CompilationError;
use crate::errors::Error;
use crate::eval_context::Config;
use crate::eval_context::ContextState;
use anyhow::anyhow;
use anyhow::Context;
use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
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
        let output = config.cargo_command("check").output();

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
        let command = config.cargo_command("build");
        if config.time_passes && config.toolchain != "nightly" {
            bail!("time_passes option requires nightly compiler");
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
        if config.cache_bytes() > 0 {
            crate::module::cache::cleanup(config.cache_bytes())?;
        }
        // Every time we compile, the output file is the same. We need to rename it so that we have
        // a unique filename, otherwise we wouldn't be able to load the result of the next
        // compilation. Also, on Windows, a loaded dll gets locked, so we couldn't even compile a
        // second time if we didn't load a different file.
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
    }

    Ok(output.status.code().unwrap_or(-1))
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
    command.arg("--extern").arg(core_extern);
    if !got_prefer_dynamic {
        command.arg("-C").arg("prefer-dynamic");
    }
    Ok(command)
}

fn should_force_dylibs() -> bool {
    std::env::var(crate::runtime::FORCE_DYLIB_ENV).is_ok()
}

fn map_extern_arg(ext: &str) -> OsString {
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
            so_arg.push(path);
            return so_arg;
        }
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
