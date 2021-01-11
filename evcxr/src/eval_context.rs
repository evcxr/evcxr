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

use crate::evcxr_internal_runtime;
use crate::item;
use crate::module::{Module, SoFile};
use crate::runtime;
use crate::rust_analyzer::{Completions, RustAnalyzer, VariableInfo};
use crate::{child_process::ChildProcess, use_trees::Import};
use crate::{
    code_block::UserCodeInfo,
    errors::{bail, CompilationError, Error},
};
use crate::{
    code_block::{CodeBlock, CodeKind, Segment},
    errors::SpannedMessage,
};
use crate::{crate_config::ExternalCrate, errors::Span};
use anyhow::Result;
use ra_ap_ide::TextRange;
use ra_ap_syntax::{ast, AstNode, SyntaxKind, SyntaxNode};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tempfile;

pub struct EvalContext {
    // Our tmpdir if EVCXR_TMPDIR wasn't set - Drop causes tmpdir to be cleaned up.
    _tmpdir: Option<tempfile::TempDir>,
    module: Module,
    committed_state: ContextState,
    child_process: ChildProcess,
    stdout_sender: mpsc::Sender<String>,
    analyzer: RustAnalyzer,
    initial_config: Config,
}

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub(crate) crate_dir: PathBuf,
    pub(crate) debug_mode: bool,
    // Whether we should preserve variables that are Copy when a panic occurs.
    // Sounds good, but unfortunately doing so currently requires an extra build
    // attempt to determine if the type of the variable is copy.
    preserve_vars_on_panic: bool,
    output_format: String,
    /// Whether to try to display the final expression. Currently this needs to
    /// be turned off when doing tab completion or cargo check, but otherwise it
    /// should always be on.
    display_final_expression: bool,
    /// Whether to expand and deduplicate use statements. We need to be able to
    /// turn this off in order for tab-completion of use statements to work, but
    /// otherwise this should always be on.
    expand_use_statements: bool,
    opt_level: String,
    error_fmt: &'static ErrorFormat,
    /// Whether to pass -Ztime-passes to the compiler and print the result.
    /// Causes the nightly compiler, which must be installed to be selected.
    pub(crate) time_passes: bool,
    pub(crate) linker: String,
    pub(crate) sccache: Option<PathBuf>,
    /// Whether to attempt to avoid network access.
    pub(crate) offline_mode: bool,
    pub(crate) toolchain: String,
}

fn create_initial_config(crate_dir: PathBuf) -> Config {
    let mut config = Config::new(crate_dir);
    if !cfg!(target_os = "macos") && which::which("lld").is_ok() {
        config.linker = "lld".to_owned();
    }
    config
}

impl Config {
    pub fn new(crate_dir: PathBuf) -> Self {
        Config {
            crate_dir,
            debug_mode: false,
            preserve_vars_on_panic: false,
            output_format: "{:?}".to_owned(),
            display_final_expression: true,
            expand_use_statements: true,
            opt_level: "2".to_owned(),
            error_fmt: &ERROR_FORMATS[0],
            time_passes: false,
            linker: "system".to_owned(),
            sccache: None,
            offline_mode: false,
            toolchain: String::new(),
        }
    }

    pub fn set_sccache(&mut self, enabled: bool) -> Result<(), Error> {
        if enabled {
            if let Ok(path) = which::which("sccache") {
                self.sccache = Some(path);
            } else {
                bail!("Couldn't find sccache. Try running `cargo install sccache`.");
            }
        } else {
            self.sccache = None;
        }
        Ok(())
    }

    pub fn sccache(&self) -> bool {
        self.sccache.is_some()
    }

    pub(crate) fn cargo_command(&self, command_name: &str) -> std::process::Command {
        let mut command = std::process::Command::new("cargo");
        if !self.toolchain.is_empty() {
            command.arg(format!("+{}", self.toolchain));
        }
        if self.offline_mode {
            command.arg("--offline");
        }
        command.arg(command_name);
        command.current_dir(&self.crate_dir);
        command
    }
}

#[derive(Debug)]
struct ErrorFormat {
    format_str: &'static str,
    format_trait: &'static str,
}

static ERROR_FORMATS: &[ErrorFormat] = &[
    ErrorFormat {
        format_str: "{}",
        format_trait: "std::fmt::Display",
    },
    ErrorFormat {
        format_str: "{:?}",
        format_trait: "std::fmt::Debug",
    },
    ErrorFormat {
        format_str: "{:#?}",
        format_trait: "std::fmt::Debug",
    },
];

const SEND_TEXT_PLAIN_DEF: &str = stringify!(
    fn evcxr_send_text_plain(text: &str) {
        use std::io::{self, Write};
        fn try_send_text(text: &str) -> io::Result<()> {
            let stdout = io::stdout();
            let mut output = stdout.lock();
            output.write_all(b"EVCXR_BEGIN_CONTENT text/plain\n")?;
            output.write_all(text.as_bytes())?;
            output.write_all(b"\nEVCXR_END_CONTENT\n")?;
            Ok(())
        }
        if let Err(error) = try_send_text(text) {
            eprintln!("Failed to send content to parent: {:?}", error);
            std::process::exit(1);
        }
    }
);

const PANIC_NOTIFICATION: &str = "EVCXR_PANIC_NOTIFICATION";

// Outputs from an EvalContext. This is a separate struct since users may want
// destructure this and pass its components to separate threads.
pub struct EvalContextOutputs {
    pub stdout: mpsc::Receiver<String>,
    pub stderr: mpsc::Receiver<String>,
}

//#[non_exhaustive]
pub struct EvalCallbacks<'a> {
    pub input_reader: &'a dyn Fn(&str, bool) -> String,
}

fn default_input_reader(_: &str, _: bool) -> String {
    String::new()
}

impl<'a> Default for EvalCallbacks<'a> {
    fn default() -> Self {
        EvalCallbacks {
            input_reader: &default_input_reader,
        }
    }
}

impl EvalContext {
    pub fn new() -> Result<(EvalContext, EvalContextOutputs), Error> {
        let current_exe = std::env::current_exe()?;
        Self::with_subprocess_command(std::process::Command::new(&current_exe))
    }

    #[cfg(windows)]
    fn apply_platform_specific_vars(module: &Module, command: &mut std::process::Command) {
        // Windows doesn't support rpath, so we need to set PATH so that it
        // knows where to find dlls.
        use std::ffi::OsString;
        let mut path_var_value = OsString::new();
        path_var_value.push(&module.deps_dir());
        path_var_value.push(";");

        let mut sysroot_command = std::process::Command::new("rustc");
        sysroot_command.arg("--print").arg("sysroot");
        path_var_value.push(format!(
            "{}\\bin;",
            String::from_utf8_lossy(&sysroot_command.output().unwrap().stdout).trim()
        ));
        path_var_value.push(std::env::var("PATH").unwrap_or_default());

        command.env("PATH", path_var_value);
    }

    #[cfg(not(windows))]
    fn apply_platform_specific_vars(_module: &Module, _command: &mut std::process::Command) {}

    #[doc(hidden)]
    pub fn new_for_testing() -> (EvalContext, EvalContextOutputs) {
        let testing_runtime_path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("testing_runtime");
        let (mut context, outputs) =
            EvalContext::with_subprocess_command(std::process::Command::new(&testing_runtime_path))
                .unwrap();
        let mut state = context.state();
        state.set_offline_mode(true);
        context.commit_state(state);
        (context, outputs)
    }

    pub fn with_subprocess_command(
        mut subprocess_command: std::process::Command,
    ) -> Result<(EvalContext, EvalContextOutputs), Error> {
        let mut opt_tmpdir = None;
        let tmpdir_path;
        if let Ok(from_env) = std::env::var("EVCXR_TMPDIR") {
            tmpdir_path = PathBuf::from(from_env);
        } else {
            let tmpdir = tempfile::tempdir()?;
            tmpdir_path = PathBuf::from(tmpdir.path());
            opt_tmpdir = Some(tmpdir);
        }

        let analyzer = RustAnalyzer::new(&tmpdir_path)?;
        let module = Module::new(tmpdir_path)?;

        Self::apply_platform_specific_vars(&module, &mut subprocess_command);

        let (stdout_sender, stdout_receiver) = mpsc::channel();
        let (stderr_sender, stderr_receiver) = mpsc::channel();
        let child_process = ChildProcess::new(subprocess_command, stderr_sender)?;
        let initial_config = create_initial_config(module.crate_dir().to_owned());
        let initial_state = ContextState::new(initial_config.clone());
        let mut context = EvalContext {
            _tmpdir: opt_tmpdir,
            committed_state: initial_state,
            module,
            child_process,
            stdout_sender,
            analyzer,
            initial_config,
        };
        let outputs = EvalContextOutputs {
            stdout: stdout_receiver,
            stderr: stderr_receiver,
        };
        if context.committed_state.linker() == "lld" && context.eval("42", context.state()).is_err()
        {
            context.committed_state.set_linker("system".to_owned());
        } else {
            // We need to eval something anyway, otherwise rust-analyzer crashes when trying to get
            // completions. Not 100% sure. Just writing Cargo.toml isn't sufficient.
            context.eval("42", context.state())?;
        }
        context.initial_config = context.committed_state.config.clone();
        Ok((context, outputs))
    }

    /// Returns a new context state, suitable for passing to `eval` after
    /// optionally calling things like `add_dep`.
    pub fn state(&self) -> ContextState {
        self.committed_state.clone()
    }

    /// Evaluates the supplied Rust code.
    pub fn eval(&mut self, code: &str, state: ContextState) -> Result<EvalOutputs, Error> {
        let (user_code, code_info) = CodeBlock::from_original_user_code(code);
        self.eval_with_callbacks(user_code, state, &code_info, &mut EvalCallbacks::default())
    }

    pub(crate) fn check(
        &mut self,
        user_code: CodeBlock,
        mut state: ContextState,
        code_info: &UserCodeInfo,
    ) -> Result<Vec<CompilationError>, Error> {
        state.config.display_final_expression = false;
        state.config.expand_use_statements = false;
        let user_code = state.apply(user_code, &code_info.nodes)?;
        let code = state.analysis_code(user_code.clone());
        let errors = self.module.check(&code, &state.config)?;
        Ok(state.apply_custom_errors(errors, &user_code, code_info))
    }

    /// Evaluates the supplied Rust code.
    pub(crate) fn eval_with_callbacks(
        &mut self,
        user_code: CodeBlock,
        mut state: ContextState,
        code_info: &UserCodeInfo,
        callbacks: &mut EvalCallbacks,
    ) -> Result<EvalOutputs, Error> {
        if user_code.is_empty()
            && !self
                .committed_state
                .state_change_can_fail_compilation(&state)
        {
            self.commit_state(state);
            return Ok(EvalOutputs::default());
        }
        let mut phases = PhaseDetailsBuilder::new();
        let code_out = state.apply(user_code.clone(), &code_info.nodes)?;

        let mut outputs = match self.run_statements(code_out, &mut state, &mut phases, callbacks) {
            error @ Err(Error::ChildProcessTerminated(_)) => {
                self.restart_child_process()?;
                return error;
            }
            Err(Error::CompilationErrors(errors)) => {
                let mut errors = state.apply_custom_errors(errors, &user_code, code_info);
                // If we have any errors in user code then remove all errors that aren't from user
                // code.
                if errors.iter().any(|error| error.is_from_user_code()) {
                    errors.retain(|error| error.is_from_user_code())
                }
                return Err(Error::CompilationErrors(errors));
            }
            error @ Err(_) => return error,
            Ok(x) => x,
        };

        // Once, we reach here, our code has successfully executed, so we
        // conclude that variable changes are now applied.
        self.commit_state(state);

        phases.phase_complete("Execution");
        outputs.phases = phases.phases;

        Ok(outputs)
    }

    pub(crate) fn completions(
        &mut self,
        user_code: CodeBlock,
        mut state: ContextState,
        nodes: &[SyntaxNode],
        offset: usize,
    ) -> Result<Completions> {
        // Wrapping the final expression in order to display it might interfere
        // with completions on that final expression.
        state.config.display_final_expression = false;
        // Expanding use statements would prevent us from tab-completing those
        // use statements, since we lose information about where each bit came
        // from when we expand. This could be fixed with some work, but there's
        // not really any downside to turn it off here. It'll produce errors,
        // but those errors don't effect the analysis needed for completions.
        state.config.expand_use_statements = false;
        let user_code = state.apply(user_code, nodes)?;
        let code = state.analysis_code(user_code);
        let wrapped_offset = code.user_offset_to_output_offset(offset)?;

        if state.config.debug_mode {
            let mut s = code.code_string();
            s.insert_str(wrapped_offset, "<|>");
            println!("=========\n{}\n==========", s);
        }

        self.analyzer.set_source(code.code_string())?;
        let mut completions = self.analyzer.completions(wrapped_offset)?;
        completions.start_offset = code.output_offset_to_user_offset(completions.start_offset)?;
        completions.end_offset = code.output_offset_to_user_offset(completions.end_offset)?;
        // Filter internal identifiers.
        completions.completions = completions
            .completions
            .into_iter()
            .filter(|c| c.code != "evcxr_variable_store" && c.code != "evcxr_internal_runtime")
            .collect();
        Ok(completions)
    }

    pub fn last_source(&self) -> Result<String, std::io::Error> {
        self.module.last_source()
    }

    pub fn set_opt_level(&mut self, level: &str) -> Result<(), Error> {
        self.committed_state.set_opt_level(level)
    }

    pub fn set_time_passes(&mut self, value: bool) {
        self.committed_state.set_time_passes(value);
    }

    pub fn set_preserve_vars_on_panic(&mut self, value: bool) {
        self.committed_state.set_preserve_vars_on_panic(value);
    }

    pub fn set_error_format(&mut self, value: &str) -> Result<(), Error> {
        self.committed_state.set_error_format(value)
    }

    pub fn variables_and_types(&self) -> impl Iterator<Item = (&str, &str)> {
        self.committed_state
            .variable_states
            .iter()
            .map(|(v, t)| (v.as_str(), t.type_name.as_str()))
    }

    pub fn defined_item_names(&self) -> impl Iterator<Item = &str> {
        self.committed_state
            .items_by_name
            .keys()
            .map(String::as_str)
    }

    // Clears all state, while keeping tmpdir. This allows us to effectively
    // restart, but without having to recompile any external crates we'd already
    // compiled. Config is preserved.
    pub fn clear(&mut self) -> Result<(), Error> {
        self.committed_state = self.cleared_state();
        self.restart_child_process()
    }

    /// Returns the state that would result from clearing. Config is preserved. Nothing is done to
    /// the subprocess.
    pub(crate) fn cleared_state(&self) -> ContextState {
        ContextState::new(self.committed_state.config.clone())
    }

    pub fn reset_config(&mut self) {
        self.committed_state.config = self.initial_config.clone();
    }

    fn restart_child_process(&mut self) -> Result<(), Error> {
        self.committed_state.variable_states.clear();
        self.committed_state.stored_variable_states.clear();
        self.child_process = self.child_process.restart()?;
        Ok(())
    }

    pub(crate) fn last_compile_dir(&self) -> &Path {
        self.module.crate_dir()
    }

    fn commit_state(&mut self, mut state: ContextState) {
        for variable_state in state.variable_states.values_mut() {
            // This span only makes sense when the variable is first defined.
            variable_state.definition_span = None;
        }
        state.stored_variable_states = state.variable_states.clone();
        state.commit_old_user_code();
        self.committed_state = state;
    }

    fn run_statements(
        &mut self,
        mut user_code: CodeBlock,
        state: &mut ContextState,
        phases: &mut PhaseDetailsBuilder,
        callbacks: &mut EvalCallbacks,
    ) -> Result<EvalOutputs, Error> {
        self.write_cargo_toml(state)?;
        self.fix_variable_types(state, state.analysis_code(user_code.clone()))?;
        // In some circumstances we may need a few tries before we get the code right. Note that
        // we'll generally give up sooner than this if there's nothing left that we think we can
        // fix. The limit is really to prevent retrying indefinitely in case our "fixing" of things
        // somehow ends up flip-flopping back and forth. Not sure how that could happen, but best to
        // avoid any infinite loops.
        let mut remaining_retries = 5;
        // TODO: Now that we have rust analyzer, we can probably with a bit of work obtain all the
        // information we need without relying on compilation errors. See if we can get rid of this.
        loop {
            // Try to compile and run the code.
            let result = self.try_run_statements(
                user_code.clone(),
                state,
                state.compilation_mode(),
                phases,
                callbacks,
            );
            match result {
                Ok(execution_artifacts) => {
                    return Ok(execution_artifacts.output);
                }

                Err(Error::CompilationErrors(errors)) => {
                    // If we failed to compile, attempt to deal with the first
                    // round of compilation errors by adjusting variable types,
                    // whether they've been moved into the catch_unwind block
                    // etc.
                    if remaining_retries > 0 {
                        let mut fixed = HashSet::new();
                        for error in &errors {
                            self.attempt_to_fix_error(error, &mut user_code, state, &mut fixed)?;
                        }
                        if !fixed.is_empty() {
                            remaining_retries -= 1;
                            let fixed_sorted: Vec<_> = fixed.into_iter().collect();
                            phases.phase_complete(&fixed_sorted.join("|"));
                            continue;
                        }
                    }
                    if !user_code.is_empty() {
                        // We have user code and it appears to have an error, recompile without
                        // catch_unwind to try and get a better error message. e.g. we don't want the
                        // user to see messages like "cannot borrow immutable captured outer variable in
                        // an `FnOnce` closure `a` as mutable".
                        self.try_run_statements(
                            user_code,
                            state,
                            CompilationMode::NoCatchExpectError,
                            phases,
                            callbacks,
                        )?;
                    }
                    return Err(Error::CompilationErrors(errors));
                }

                Err(Error::TypeRedefinedVariablesLost(variables)) => {
                    for variable in &variables {
                        state.variable_states.remove(variable);
                        state.stored_variable_states.remove(variable);
                        self.committed_state.variable_states.remove(variable);
                        self.committed_state.stored_variable_states.remove(variable);
                    }
                    remaining_retries -= 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn try_run_statements(
        &mut self,
        user_code: CodeBlock,
        state: &mut ContextState,
        compilation_mode: CompilationMode,
        phases: &mut PhaseDetailsBuilder,
        callbacks: &mut EvalCallbacks,
    ) -> Result<ExecutionArtifacts, Error> {
        let code = state.code_to_compile(user_code, compilation_mode);
        let so_file = self.module.compile(&code, &state.config)?;

        if compilation_mode == CompilationMode::NoCatchExpectError {
            // Uh-oh, caller was expecting an error, return OK and the caller can return the
            // original error.
            return Ok(ExecutionArtifacts {
                output: EvalOutputs::new(),
            });
        }
        phases.phase_complete("Final compile");

        let output = self.run_and_capture_output(state, &so_file, callbacks)?;
        Ok(ExecutionArtifacts { output })
    }

    pub(crate) fn write_cargo_toml(&self, state: &ContextState) -> Result<()> {
        self.module.write_cargo_toml(state)?;
        Ok(())
    }

    fn fix_variable_types(
        &mut self,
        state: &mut ContextState,
        code: CodeBlock,
    ) -> Result<(), Error> {
        self.analyzer.set_source(code.code_string())?;
        for (
            variable_name,
            VariableInfo {
                type_name,
                is_mutable,
            },
        ) in self.analyzer.top_level_variables("evcxr_analysis_wrapper")
        {
            // For now, we need to look for and escape any reserved words. This should probably in
            // theory be done in rust analyzer in a less hacky way.
            let type_name = replace_reserved_words_in_type(&type_name);
            // We don't want to try to store record evcxr_variable_store into itself, so we ignore
            // it. We also ignore any variables for which we were given an invalid type. Variables
            // with invalid types will then have their types determined by looking at compilation
            // errors (although we may eventually drop the code that does that). At the time of
            // writing, the test `int_array` fails if we don't reject invalid types here.
            if variable_name == "evcxr_variable_store"
                || !crate::rust_analyzer::is_type_valid(&type_name)
            {
                continue;
            }
            let preserve_vars_on_panic = state.config.preserve_vars_on_panic;
            state
                .variable_states
                .entry(variable_name)
                .or_insert_with(|| VariableState {
                    type_name: String::new(),
                    is_mut: is_mutable,
                    // All new locals will initially be defined only inside our catch_unwind
                    // block.
                    move_state: VariableMoveState::MovedIntoCatchUnwind,
                    // If we're preserving copy types, then assume this variable
                    // is copy until we find out it's not.
                    is_copy_type: preserve_vars_on_panic,
                    definition_span: None,
                })
                .type_name = type_name;
        }
        Ok(())
    }

    fn run_and_capture_output(
        &mut self,
        state: &mut ContextState,
        so_file: &SoFile,
        callbacks: &mut EvalCallbacks,
    ) -> Result<EvalOutputs, Error> {
        let mut output = EvalOutputs::new();
        // TODO: We should probably send an OsString not a String. Otherwise
        // things won't work if the path isn't UTF-8 - apparently that's a thing
        // on some platforms.
        let fn_name = state.current_user_fn_name();
        self.child_process.send(&format!(
            "LOAD_AND_RUN {} {}",
            so_file.path.to_string_lossy(),
            fn_name,
        ))?;

        state.build_num += 1;

        let mut got_panic = false;
        let mut lost_variables = Vec::new();
        lazy_static! {
            static ref MIME_OUTPUT: Regex = Regex::new("EVCXR_BEGIN_CONTENT ([^ ]+)").unwrap();
        }
        loop {
            let line = self.child_process.recv_line()?;
            if line == runtime::EVCXR_EXECUTION_COMPLETE {
                break;
            }
            if line == PANIC_NOTIFICATION {
                got_panic = true;
            } else if line.starts_with(evcxr_input::GET_CMD) {
                let is_password = line.starts_with(evcxr_input::GET_CMD_PASSWORD);
                let prompt = line.split(':').skip(1).next().unwrap_or_default();
                self.child_process
                    .send(&(callbacks.input_reader)(prompt, is_password))?;
            } else if line == evcxr_internal_runtime::USER_ERROR_OCCURRED {
                // A question mark operator in user code triggered an early
                // return. Any variables moved into the block in which the code
                // was running, including any newly defined variables will have
                // been lost (or possibly never even defined).
                state
                    .variable_states
                    .retain(|_variable_name, variable_state| {
                        variable_state.move_state != VariableMoveState::MovedIntoCatchUnwind
                    });
            } else if line.starts_with(evcxr_internal_runtime::VARIABLE_CHANGED_TYPE) {
                let variable_name = &line[evcxr_internal_runtime::VARIABLE_CHANGED_TYPE.len()..];
                lost_variables.push(variable_name.to_owned());
            } else if let Some(captures) = MIME_OUTPUT.captures(&line) {
                let mime_type = captures[1].to_owned();
                let mut content = String::new();
                loop {
                    let line = self.child_process.recv_line()?;
                    if line == "EVCXR_END_CONTENT" {
                        break;
                    }
                    if line == PANIC_NOTIFICATION {
                        got_panic = true;
                        break;
                    }
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&line);
                }
                output.content_by_mime_type.insert(mime_type, content);
            } else {
                // Note, errors sending are ignored, since it just means the
                // user of the library has dropped the Receiver.
                // TODO: Clone might not be necessary with NLL.
                let _ = self.stdout_sender.send(line.clone());
            }
        }
        if got_panic {
            let mut lost = Vec::new();
            state
                .variable_states
                .retain(|variable_name, variable_state| {
                    if variable_state.move_state == VariableMoveState::MovedIntoCatchUnwind {
                        lost.push(variable_name.clone());
                        false
                    } else {
                        true
                    }
                });
            if !lost.is_empty() {
                output.content_by_mime_type.insert(
                    "text/plain".to_owned(),
                    format!(
                        "Panic occurred, the following variables have been lost: {}",
                        lost.join(", ")
                    ),
                );
            }
        } else if !lost_variables.is_empty() {
            return Err(Error::TypeRedefinedVariablesLost(lost_variables));
        }
        Ok(output)
    }

    fn attempt_to_fix_error(
        &mut self,
        error: &CompilationError,
        user_code: &mut CodeBlock,
        state: &mut ContextState,
        fixed_errors: &mut HashSet<&'static str>,
    ) -> Result<(), Error> {
        lazy_static! {
            static ref DISALLOWED_TYPES: Regex = Regex::new("(impl .*|[.*@])").unwrap();
        }
        for code_origin in &error.code_origins {
            match code_origin {
                CodeKind::PackVariable { variable_name } => {
                    if error.code() == Some("E0308") {
                        // Handle mismatched types. We might eventually remove this code entirely
                        // now that we use Rust analyzer for type inference. Keeping it for now as
                        // there's still a handful of tests that fail without this code..
                        if let Some(mut actual_type) = error.get_actual_type() {
                            // If the user hasn't given enough information for the compiler to
                            // determine what type of integer or float, we default to i32 and f64
                            // respectively.
                            actual_type = actual_type
                                .replace("{integer}", "i32")
                                .replace("{float}", "f64");
                            if actual_type == "integer" {
                                actual_type = "i32".to_string();
                            } else if actual_type == "float" {
                                actual_type = "f64".to_string();
                            }
                            if DISALLOWED_TYPES.is_match(&actual_type) {
                                bail!(
                                    "Sorry, the type {} cannot currently be persisted",
                                    actual_type
                                );
                            }
                            actual_type = replace_reserved_words_in_type(&actual_type);
                            state
                                .variable_states
                                .get_mut(variable_name)
                                .unwrap()
                                .type_name = actual_type;
                            fixed_errors.insert("Variable types");
                        } else {
                            bail!("Got error E0308 but failed to parse actual type");
                        }
                    } else if error.code() == Some("E0382") {
                        // Use of moved value.
                        let old_move_state = std::mem::replace(
                            &mut state
                                .variable_states
                                .get_mut(variable_name)
                                .unwrap()
                                .move_state,
                            VariableMoveState::MovedIntoCatchUnwind,
                        );
                        if old_move_state == VariableMoveState::MovedIntoCatchUnwind {
                            // Variable is truly moved, forget about it.
                            state.variable_states.remove(variable_name);
                        }
                        fixed_errors.insert("Captured value");
                    } else if error.code() == Some("E0425") {
                        // cannot find value in scope.
                        state.variable_states.remove(variable_name);
                        fixed_errors.insert("Variable moved");
                    } else if error.code() == Some("E0603") {
                        if let Some(variable_state) = state.variable_states.remove(variable_name) {
                            bail!(
                                "Failed to determine type of variable `{}`. rustc suggested type \
                             {}, but that's private. Sometimes adding an extern crate will help \
                             rustc suggest the correct public type name, or you can give an \
                             explicit type.",
                                variable_name,
                                variable_state.type_name
                            );
                        }
                    } else if error.code() == Some("E0562")
                        || (error.code().is_none() && error.code_origins.len() == 1)
                    {
                        bail!(
                            "The variable `{}` has a type `{}` that can't be persisted. You can \
                            try wrapping your code in braces so that the variable goes out of \
                            scope before the end of the code to be executed.",
                            variable_name,
                            state.variable_states[variable_name].type_name
                        );
                    }
                }
                CodeKind::AssertCopyType { variable_name } => {
                    if error.code() == Some("E0277") {
                        if let Some(variable_state) = state.variable_states.get_mut(variable_name) {
                            variable_state.is_copy_type = false;
                            fixed_errors.insert("Non-copy type");
                        }
                    }
                }
                CodeKind::WithFallback(fallback) => {
                    user_code.apply_fallback(fallback);
                    fixed_errors.insert("Fallback");
                }
                CodeKind::OriginalUserCode(_) | CodeKind::OtherUserCode => {
                    if error.code() == Some("E0728") && !state.async_mode {
                        state.async_mode = true;
                        if !state.external_deps.contains_key("tokio") {
                            state.add_dep("tokio", "\"0.2\"")?;
                        }
                        fixed_errors.insert("Enabled async mode");
                    } else if error.code() == Some("E0277") && !state.allow_question_mark {
                        state.allow_question_mark = true;
                        fixed_errors.insert("Allow question mark");
                    } else if error.code() == Some("E0658")
                        && error
                            .message()
                            .contains("`let` expressions in this position are experimental")
                    {
                        // PR to add a semicolon is welcome. Ideally we'd not do so here though. It
                        // should really be done based on the parse tree of the code. We currently
                        // have two parsers, syn and rust-analyzer. We'd like to eventually get rid
                        // of syn and just user rust-analyzer, but the code that could potentially
                        // add a semicolon currently uses syn. So ideally we'd replace uses of syn
                        // with rust-analyzer before adding new parse-tree based rules. But PRs that
                        // just use syn to determine when to add a semicolon would also be OK.
                        bail!("Looks like you're missing a semicolon");
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Returns whether a type is fully specified. i.e. it doesn't contain any '_'.
fn type_is_fully_specified(ty: &ast::Type) -> bool {
    !AstNode::syntax(ty)
        .descendants()
        .any(|n| n.kind() == SyntaxKind::INFER_TYPE)
}

#[derive(Debug)]
pub struct PhaseDetails {
    pub name: String,
    pub duration: Duration,
}

struct PhaseDetailsBuilder {
    start: Instant,
    phases: Vec<PhaseDetails>,
}

impl PhaseDetailsBuilder {
    fn new() -> PhaseDetailsBuilder {
        PhaseDetailsBuilder {
            start: Instant::now(),
            phases: Vec::new(),
        }
    }

    fn phase_complete(&mut self, name: &str) {
        let new_start = Instant::now();
        self.phases.push(PhaseDetails {
            name: name.to_owned(),
            duration: new_start.duration_since(self.start),
        });
        self.start = new_start;
    }
}

#[derive(Default, Debug)]
pub struct EvalOutputs {
    pub content_by_mime_type: HashMap<String, String>,
    pub timing: Option<Duration>,
    pub phases: Vec<PhaseDetails>,
}

impl EvalOutputs {
    pub fn new() -> EvalOutputs {
        EvalOutputs {
            content_by_mime_type: HashMap::new(),
            timing: None,
            phases: Vec::new(),
        }
    }

    pub fn text_html(text: String, html: String) -> EvalOutputs {
        let mut out = EvalOutputs::new();
        out.content_by_mime_type
            .insert("text/plain".to_owned(), text);
        out.content_by_mime_type
            .insert("text/html".to_owned(), html);
        out
    }

    pub fn is_empty(&self) -> bool {
        self.content_by_mime_type.is_empty()
    }

    pub fn get(&self, mime_type: &str) -> Option<&str> {
        self.content_by_mime_type.get(mime_type).map(String::as_str)
    }

    pub fn merge(&mut self, other: EvalOutputs) {
        for (mime_type, content) in other.content_by_mime_type {
            self.content_by_mime_type
                .entry(mime_type)
                .or_default()
                .push_str(&content);
        }
    }
}

#[derive(Clone, Debug)]
struct VariableState {
    type_name: String,
    is_mut: bool,
    move_state: VariableMoveState,
    // Whether the type of this variable implements Copy. Variables that implement copy never get
    // moved into the catch_unwind block (they get copied), so we need to make sure we always save
    // them from within the catch_unwind block, otherwise any changes made to the variable within
    // the block will be lost.
    is_copy_type: bool,
    definition_span: Option<UserCodeSpan>,
}

#[derive(Clone, Debug)]
struct UserCodeSpan {
    segment_index: usize,
    range: TextRange,
}

#[derive(PartialEq, Eq, Debug, Clone)]
enum VariableMoveState {
    Available,
    CopiedIntoCatchUnwind,
    MovedIntoCatchUnwind,
}

struct ExecutionArtifacts {
    output: EvalOutputs,
}

#[derive(Eq, PartialEq, Copy, Clone)]
enum CompilationMode {
    /// User code should be wrapped in catch_unwind and executed.
    RunAndCatchPanics,
    /// User code should be executed without a catch_unwind.
    NoCatch,
    /// Recompile without catch_unwind to try to get better error messages. If compilation succeeds
    /// (hopefully can't happen), don't run the code - caller should return the original message.
    NoCatchExpectError,
}

/// State that is cloned then modified every time we try to compile some code. If compilation
/// succeeds, we keep the modified state, if it fails, we revert to the old state.
#[derive(Clone, Debug)]
pub struct ContextState {
    items_by_name: HashMap<String, CodeBlock>,
    unnamed_items: Vec<CodeBlock>,
    pub(crate) external_deps: HashMap<String, ExternalCrate>,
    // Keyed by crate name. Could use a set, except that the statement might be
    // formatted slightly differently.
    extern_crate_stmts: HashMap<String, String>,
    /// States of variables. Includes variables that have just been defined by
    /// the code about to be executed.
    variable_states: HashMap<String, VariableState>,
    /// State of variables that have been stored. i.e. after the last bit of
    /// code was executed. Doesn't include newly defined variables until after
    /// execution completes.
    stored_variable_states: HashMap<String, VariableState>,
    async_mode: bool,
    allow_question_mark: bool,
    build_num: i32,
    config: Config,
}

impl ContextState {
    fn new(config: Config) -> ContextState {
        ContextState {
            items_by_name: HashMap::new(),
            unnamed_items: vec![],
            external_deps: HashMap::new(),
            extern_crate_stmts: HashMap::new(),
            variable_states: HashMap::new(),
            stored_variable_states: HashMap::new(),
            async_mode: false,
            allow_question_mark: false,
            build_num: 0,
            config,
        }
    }

    pub fn time_passes(&self) -> bool {
        self.config.time_passes
    }

    pub fn set_time_passes(&mut self, value: bool) {
        self.config.time_passes = value;
    }

    pub fn set_offline_mode(&mut self, value: bool) {
        self.config.offline_mode = value;
    }

    pub fn set_sccache(&mut self, enabled: bool) -> Result<(), Error> {
        self.config.set_sccache(enabled)
    }

    pub fn sccache(&self) -> bool {
        self.config.sccache()
    }

    pub fn set_error_format(&mut self, format_str: &str) -> Result<(), Error> {
        for format in ERROR_FORMATS {
            if format.format_str == format_str {
                self.config.error_fmt = format;
                return Ok(());
            }
        }
        bail!(
            "Unsupported error format string. Available options: {}",
            ERROR_FORMATS
                .iter()
                .map(|f| f.format_str)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    pub fn error_format(&self) -> &str {
        self.config.error_fmt.format_str
    }

    pub fn error_format_trait(&self) -> &str {
        self.config.error_fmt.format_trait
    }

    pub fn set_linker(&mut self, linker: String) {
        self.config.linker = linker;
    }

    pub fn linker(&self) -> &str {
        &self.config.linker
    }

    pub fn preserve_vars_on_panic(&self) -> bool {
        self.config.preserve_vars_on_panic
    }

    pub fn offline_mode(&mut self) -> bool {
        self.config.offline_mode
    }

    pub fn set_preserve_vars_on_panic(&mut self, value: bool) {
        self.config.preserve_vars_on_panic = value;
    }

    pub fn debug_mode(&self) -> bool {
        self.config.debug_mode
    }

    pub fn set_debug_mode(&mut self, debug_mode: bool) {
        self.config.debug_mode = debug_mode;
    }

    pub fn opt_level(&self) -> &str {
        &self.config.opt_level
    }

    pub fn set_opt_level(&mut self, level: &str) -> Result<(), Error> {
        if level.is_empty() {
            bail!("Optimization level cannot be an empty string");
        }
        self.config.opt_level = level.to_owned();
        Ok(())
    }
    pub fn output_format(&self) -> &str {
        &self.config.output_format
    }

    pub fn set_output_format(&mut self, output_format: String) {
        self.config.output_format = output_format;
    }

    pub fn set_toolchain(&mut self, value: &str) {
        self.config.toolchain = value.to_owned();
    }

    pub fn toolchain(&mut self) -> &str {
        &self.config.toolchain
    }

    /// Adds a crate dependency with the specified name and configuration.
    pub fn add_dep(&mut self, dep: &str, dep_config: &str) -> Result<(), Error> {
        // Avoid repeating dep validation once we're already added it.
        if let Some(existing) = self.external_deps.get(dep) {
            if existing.config == dep_config {
                return Ok(());
            }
        }
        crate::cargo_metadata::validate_dep(dep, dep_config, &self.config)?;
        self.external_deps.insert(
            dep.to_owned(),
            ExternalCrate::new(dep.to_owned(), dep_config.to_owned())?,
        );
        Ok(())
    }

    /// Clears fields that aren't useful for inclusion in bug reports and which might give away
    /// things like usernames.
    pub(crate) fn clear_non_debug_relevant_fields(&mut self) {
        self.config.crate_dir = PathBuf::from("redacted");
        if self.config.sccache.is_some() {
            self.config.sccache = Some(PathBuf::from("redacted"));
        }
    }

    fn apply_custom_errors(
        &self,
        errors: Vec<CompilationError>,
        user_code: &CodeBlock,
        code_info: &UserCodeInfo,
    ) -> Vec<CompilationError> {
        errors
            .into_iter()
            .filter_map(|error| self.customize_error(error, user_code))
            .map(|mut error| {
                error.fill_lines(code_info);
                error
            })
            .collect()
    }

    /// Customizes errors based on their origins.
    fn customize_error(
        &self,
        error: CompilationError,
        user_code: &CodeBlock,
    ) -> Option<CompilationError> {
        for origin in &error.code_origins {
            if let CodeKind::PackVariable { variable_name } = origin {
                if let Some(definition_span) = &self.variable_states[variable_name].definition_span
                {
                    if let Some(segment) =
                        user_code.segment_with_index(definition_span.segment_index)
                    {
                        if let Some(span) = Span::from_segment(segment, definition_span.range) {
                            return self.replacement_for_pack_variable_error(
                                variable_name,
                                span,
                                segment,
                                &error,
                            );
                        }
                    }
                }
            }
        }
        Some(error)
    }

    fn replacement_for_pack_variable_error(
        &self,
        variable_name: &str,
        variable_span: Span,
        segment: &Segment,
        error: &CompilationError,
    ) -> Option<CompilationError> {
        let message = match error.code().unwrap_or("") {
            "E0382" | "E0505" => {
                // Value used after move. When we go to execute the code, we'll detect this error and
                // remove the variable so it doesn't get stored.
                return None;
            }
            "E0597" => {
                format!(
                    "The variable `{}` contains a reference with a non-static lifetime so can't be persisted",
                    variable_name
                )
            }
            _ => {
                return Some(error.clone());
            }
        };
        Some(CompilationError::from_segment_span(
            segment,
            SpannedMessage::from_segment_span(segment, variable_span),
            message,
        ))
    }

    /// Returns whether transitioning to `new_state` might cause compilation
    /// failures. e.g. if `new_state` has extra dependencies, then we must
    /// return true. If we return false, we're saying that the proposed state
    /// change cannot cause compilation failures, so compilation can be skipped
    /// if there is otherwise no code to execute.
    fn state_change_can_fail_compilation(&self, new_state: &ContextState) -> bool {
        (self.extern_crate_stmts != new_state.extern_crate_stmts
            && !new_state.extern_crate_stmts.is_empty())
            || (self.external_deps != new_state.external_deps
                && !new_state.external_deps.is_empty())
            || (self.items_by_name != new_state.items_by_name
                && !new_state.items_by_name.is_empty())
            || (self.config.sccache != new_state.config.sccache)
    }

    pub(crate) fn format_cargo_deps(&self) -> String {
        self.external_deps
            .values()
            .map(|krate| format!("{} = {}\n", krate.name, krate.config))
            .collect::<Vec<_>>()
            .join("")
    }

    fn compilation_mode(&self) -> CompilationMode {
        if self.config.preserve_vars_on_panic {
            CompilationMode::RunAndCatchPanics
        } else {
            CompilationMode::NoCatch
        }
    }

    /// Returns code suitable for analysis purposes. Doesn't attempt to preserve runtime behavior.
    fn analysis_code(&self, user_code: CodeBlock) -> CodeBlock {
        let mut code = CodeBlock::new()
            .generated("#![allow(unused_imports, unused_mut, dead_code)]")
            .add_all(self.items_code())
            .add_all(self.error_trait_code(true))
            .generated("fn evcxr_variable_store<T: 'static>(_: T) {}")
            .generated("#[allow(unused_variables)]")
            .generated("fn evcxr_analysis_wrapper(");
        for (var_name, state) in &self.stored_variable_states {
            code = code.generated(format!(
                "{}{}: {},",
                if state.is_mut { "mut " } else { "" },
                var_name,
                state.type_name
            ));
        }
        code = code
            .generated(") -> Result<(), EvcxrUserCodeError> {")
            .add_all(user_code);

        // Pack variable statements in analysis mode are a lot simpler than in compiled mode. We
        // just call a function that enforces that the variable doesn't contain any non-static
        // lifetimes.
        for (var_name, _var_state) in &self.variable_states {
            code.pack_variable(
                var_name.clone(),
                format!("evcxr_variable_store({});", var_name),
            );
        }

        code = code.generated("Ok(())").generated("}");
        code
    }

    fn code_to_compile(
        &self,
        user_code: CodeBlock,
        compilation_mode: CompilationMode,
    ) -> CodeBlock {
        let mut code = CodeBlock::new()
            .generated("#![allow(unused_imports, unused_mut, dead_code)]")
            .add_all(self.items_code());
        let has_user_code = !user_code.is_empty();
        if has_user_code {
            code = code.add_all(self.wrap_user_code(user_code, compilation_mode));
        } else {
            // TODO: Add a mechanism to load a crate without any function to call then remove this.
            code = code
                .generated("#[no_mangle]")
                .generated(format!(
                    "pub extern \"C\" fn {}(",
                    self.current_user_fn_name()
                ))
                .generated("mut x: *mut std::os::raw::c_void) -> *mut std::os::raw::c_void {x}");
        }
        code
    }

    fn items_code(&self) -> CodeBlock {
        let mut code = CodeBlock::new().add_all(self.get_imports());
        for item in self.items_by_name.values().chain(self.unnamed_items.iter()) {
            code = code.add_all(item.clone());
        }
        code
    }

    fn error_trait_code(&self, for_analysis: bool) -> CodeBlock {
        CodeBlock::new().generated(format!(
            r#"
            struct EvcxrUserCodeError {{}}
            impl<T: {}> From<T> for EvcxrUserCodeError {{
                fn from(error: T) -> Self {{
                    eprintln!("{}", error);
                    {}
                    EvcxrUserCodeError {{}}
                }}
            }}
        "#,
            self.config.error_fmt.format_trait,
            self.config.error_fmt.format_str,
            if for_analysis {
                ""
            } else {
                "println!(\"{}\", evcxr_internal_runtime::USER_ERROR_OCCURRED);"
            }
        ))
    }

    fn wrap_user_code(
        &self,
        mut user_code: CodeBlock,
        compilation_mode: CompilationMode,
    ) -> CodeBlock {
        let needs_variable_store = !self.variable_states.is_empty()
            || !self.stored_variable_states.is_empty()
            || self.async_mode
            || self.allow_question_mark;
        let mut code = CodeBlock::new();
        if self.allow_question_mark {
            code = code.add_all(self.error_trait_code(false));
        }
        if needs_variable_store {
            code = code
                .generated("mod evcxr_internal_runtime {")
                .generated(include_str!("evcxr_internal_runtime.rs"))
                .generated("}");
        }
        code = code.generated("#[no_mangle]").generated(format!(
            "pub extern \"C\" fn {}(",
            self.current_user_fn_name()
        ));
        if needs_variable_store {
            code = code
                .generated("mut evcxr_variable_store: *mut evcxr_internal_runtime::VariableStore)")
                .generated("  -> *mut evcxr_internal_runtime::VariableStore {")
                .generated("if evcxr_variable_store.is_null() {")
                .generated(
                    "  evcxr_variable_store = evcxr_internal_runtime::create_variable_store();",
                )
                .generated("}")
                .generated("let evcxr_variable_store = unsafe {&mut *evcxr_variable_store};")
                .add_all(self.check_variable_statements())
                .add_all(self.load_variable_statements());
            user_code = user_code
                .add_all(self.store_variable_statements(&VariableMoveState::MovedIntoCatchUnwind))
                .add_all(self.store_variable_statements(&VariableMoveState::CopiedIntoCatchUnwind));
        } else {
            code = code.generated("evcxr_variable_store: *mut u8) -> *mut u8 {");
        }
        if self.async_mode {
            user_code = CodeBlock::new()
                .generated(stringify!(evcxr_variable_store
                    .lazy_arc("evcxr_tokio_runtime", || std::sync::Mutex::new(
                        tokio::runtime::Runtime::new().unwrap()
                    ))
                    .lock()
                    .unwrap()))
                .generated(".block_on(async {")
                .add_all(user_code);
            if self.allow_question_mark {
                user_code = CodeBlock::new()
                    .generated("let _ =")
                    .add_all(user_code)
                    .generated("Ok::<(), EvcxrUserCodeError>(())");
            }
            user_code = user_code.generated("});")
        } else if self.allow_question_mark {
            user_code = CodeBlock::new()
                .generated("let _ = (|| -> std::result::Result<(), EvcxrUserCodeError> {")
                .add_all(user_code)
                .generated("Ok(())})();");
        }
        if compilation_mode == CompilationMode::RunAndCatchPanics {
            if needs_variable_store {
                code = code
                    .generated("match std::panic::catch_unwind(")
                    .generated("  std::panic::AssertUnwindSafe(move ||{")
                    // Shadow the outer evcxr_variable_store with a local one for variables moved
                    // into the closure.
                    .generated(
                        "let mut evcxr_variable_store = evcxr_internal_runtime::VariableStore::new();",
                    )
                    .add_all(user_code)
                    // Return our local variable store from the closure to be merged back into the
                    // main variable store.
                    .generated("evcxr_variable_store")
                    .generated("})) { ")
                    .generated("  Ok(inner_store) => evcxr_variable_store.merge(inner_store),")
                    .generated("  Err(_) => {")
                    .add_all(
                        self.store_variable_statements(&VariableMoveState::CopiedIntoCatchUnwind),
                    )
                    .generated(format!("    println!(\"{}\");", PANIC_NOTIFICATION))
                    .generated("}}");
            } else {
                code = code
                    .generated("if std::panic::catch_unwind(||{")
                    .add_all(user_code)
                    .generated("}).is_err() {")
                    .generated(format!("    println!(\"{}\");", PANIC_NOTIFICATION))
                    .generated("}");
            }
        } else {
            code = code.add_all(user_code);
        }
        if needs_variable_store {
            code = code.add_all(self.store_variable_statements(&VariableMoveState::Available));
        }
        code = code.generated("evcxr_variable_store");
        code.generated("}")
    }

    fn store_variable_statements(&self, move_state: &VariableMoveState) -> CodeBlock {
        let mut statements = CodeBlock::new();
        for (var_name, var_state) in &self.variable_states {
            if var_state.move_state == *move_state {
                statements.pack_variable(
                    var_name.clone(),
                    format!(
                        // Note, we use stringify instead of quoting ourselves since it results in
                        // better errors if the user forgets to close a double-quote in their code.
                        "evcxr_variable_store.put_variable::<{}>(stringify!({}), {});",
                        var_state.type_name, var_name, var_name
                    ),
                );
                if var_state.is_copy_type {
                    statements.assert_copy_variable(
                        var_name.clone(),
                        format!("evcxr_variable_store.assert_copy_type({});", var_name),
                    );
                }
            }
        }
        statements
    }

    fn check_variable_statements(&self) -> CodeBlock {
        let mut statements = CodeBlock::new().generated("{let mut vars_ok = true;");
        for (var_name, var_state) in &self.stored_variable_states {
            statements = statements.generated(format!(
                "vars_ok &= evcxr_variable_store.check_variable::<{}>(stringify!({}));",
                var_state.type_name, var_name
            ));
        }
        statements.generated("if !vars_ok {return evcxr_variable_store;}}")
    }

    // Returns code to load values from the variable store back into their variables.
    fn load_variable_statements(&self) -> CodeBlock {
        let mut statements = CodeBlock::new();
        for (var_name, var_state) in &self.stored_variable_states {
            let mutability = if var_state.is_mut { "mut " } else { "" };
            statements.load_variable(format!(
                "let {}{} = evcxr_variable_store.take_variable::<{}>(stringify!({}));",
                mutability, var_name, var_state.type_name, var_name
            ));
        }
        statements
    }

    fn current_user_fn_name(&self) -> String {
        format!("run_user_code_{}", self.build_num)
    }

    fn get_imports(&self) -> CodeBlock {
        let mut extern_stmts = CodeBlock::new();
        for stmt in self.extern_crate_stmts.values() {
            extern_stmts = extern_stmts.other_user_code(stmt.clone());
        }
        extern_stmts
    }

    /// Converts OriginalUserCode to OtherUserCode. OriginalUserCode can only be
    /// used for the current code that's being evaluated, otherwise things like
    /// tab completion will be confused, since there will be multiple bits of
    /// code at a particular offset.
    fn commit_old_user_code(&mut self) {
        for block in self.items_by_name.values_mut() {
            block.commit_old_user_code();
        }
        for block in self.unnamed_items.iter_mut() {
            block.commit_old_user_code();
        }
    }

    /// Applies `user_code` to this state object, returning the updated user
    /// code. Things like use-statements will be removed from the returned code,
    /// as they will have been stored in `self`.
    fn apply(&mut self, user_code: CodeBlock, nodes: &[SyntaxNode]) -> Result<CodeBlock, Error> {
        if self.config.preserve_vars_on_panic {
            // Any pre-existing, non-copy variables are marked as available, so that we'll take their
            // values from outside of the catch_unwind block. If they remain this way, then this
            // effectively means that they're not being used.
            for variable_state in self.variable_states.values_mut() {
                variable_state.move_state = if variable_state.is_copy_type {
                    VariableMoveState::CopiedIntoCatchUnwind
                } else {
                    VariableMoveState::Available
                };
            }
        } else {
            for variable_state in self.variable_states.values_mut() {
                variable_state.move_state = VariableMoveState::Available;
            }
        }

        let mut code_out = CodeBlock::new();
        let mut previous_item_name = None;
        let num_statements = user_code.segments.len();
        for (statement_index, segment) in user_code.segments.into_iter().enumerate() {
            let node = if let CodeKind::OriginalUserCode(meta) = &segment.kind {
                &nodes[meta.node_index]
            } else {
                code_out = code_out.with_segment(segment);
                continue;
            };
            if let Some(let_stmt) = ast::LetStmt::cast(node.clone()) {
                if let Some(pat) = let_stmt.pat() {
                    self.record_new_locals(pat, let_stmt.ty(), &segment, node.text_range());
                    code_out = code_out.with_segment(segment);
                }
            } else if ast::Expr::can_cast(node.kind()) {
                if statement_index == num_statements - 1 {
                    if self.config.display_final_expression {
                        code_out = code_out.code_with_fallback(
                            // First we try calling .evcxr_display().
                            CodeBlock::new()
                                .generated("(")
                                .with_segment(segment.clone())
                                .generated(").evcxr_display();")
                                .code_string(),
                            // If that fails, we try debug format.
                            CodeBlock::new()
                                .generated(SEND_TEXT_PLAIN_DEF)
                                .generated(&format!(
                                    "evcxr_send_text_plain(&format!(\"{}\",\n",
                                    self.config.output_format
                                ))
                                .with_segment(segment)
                                .generated("));"),
                        );
                    } else {
                        code_out = code_out
                            .generated("let _ = ")
                            .with_segment(segment)
                            .generated(";");
                    }
                } else {
                    // We got an expression, but it wasn't the last statement,
                    // so don't try to print it. Yes, this is possible. For
                    // example `for x in y {}` is an expression. See the test
                    // non_semi_statements.
                    code_out = code_out.with_segment(segment);
                }
            } else if let Some(item) = ast::Item::cast(node.clone()) {
                match item {
                    ast::Item::ExternCrate(extern_crate) => {
                        if let Some(crate_name) = extern_crate.name_ref() {
                            let crate_name = crate_name.text().to_string();
                            if !self.dependency_lib_names()?.contains(&crate_name) {
                                self.external_deps
                                    .entry(crate_name.clone())
                                    .or_insert_with(|| {
                                        ExternalCrate::new(crate_name.clone(), "\"*\"".to_owned())
                                            .unwrap()
                                    });
                            }
                            self.extern_crate_stmts
                                .insert(crate_name, segment.code.clone());
                        }
                    }
                    ast::Item::MacroRules(macro_rules) => {
                        if let Some(name) = ast::NameOwner::name(&macro_rules) {
                            let item_block = CodeBlock::new().with_segment(segment);
                            self.items_by_name
                                .insert(name.text().to_string(), item_block);
                        } else {
                            code_out = code_out.with_segment(segment);
                        }
                    }
                    ast::Item::Use(use_stmt) => {
                        if let Some(use_tree) = use_stmt.use_tree() {
                            if self.config.expand_use_statements {
                                // This mode is used for normal execution as it results in all named
                                // items being stored separately, which permits future code to
                                // deduplicate / replace those items. It doesn't however preserve
                                // traceability back to the original user's code, so isn't so useful
                                // for analysis purposes.
                                crate::use_trees::use_tree_names_do(&use_tree, &mut |import| {
                                    match import {
                                        Import::Unnamed(code) => {
                                            self.unnamed_items
                                                .push(CodeBlock::new().other_user_code(code));
                                        }
                                        Import::Named { name, code } => {
                                            self.items_by_name.insert(
                                                name,
                                                CodeBlock::new().other_user_code(code),
                                            );
                                        }
                                    }
                                });
                            } else {
                                // This mode finds all names that the use statement expands to, then
                                // removes any previous definitions of those names and then adds the
                                // original user code as-is. This allows error reporting on the
                                // added line. It's only good for one-off usage though, since all
                                // the names get put into `unnamed_items`, so can't get tracked.
                                // Fortunately this is fine for analysis purposes, since we always
                                // through away the state after we're done with analysis.
                                crate::use_trees::use_tree_names_do(&use_tree, &mut |import| {
                                    if let Import::Named { name, .. } = import {
                                        self.items_by_name.remove(&name);
                                    }
                                });
                                self.unnamed_items
                                    .push(CodeBlock::new().with_segment(segment));
                            }
                        } else {
                            // No use-tree probably means something is malformed, just put it into
                            // the output as-is so that we can get proper error reporting.
                            code_out = code_out.with_segment(segment);
                        }
                    }
                    item => {
                        let item_block = CodeBlock::new().with_segment(segment);
                        if let Some(item_name) = item::item_name(&item) {
                            *self.items_by_name.entry(item_name.to_owned()).or_default() =
                                item_block;
                            previous_item_name = Some(item_name);
                        } else if let Some(item_name) = &previous_item_name {
                            // unwrap below should never fail because we put
                            // that key in the map on a previous iteration,
                            // otherwise we wouldn't have had a value in
                            // `previous_item_name`.
                            self.items_by_name
                                .get_mut(item_name)
                                .unwrap()
                                .modify(move |block_for_name| block_for_name.add_all(item_block));
                        } else {
                            self.unnamed_items.push(item_block);
                        }
                    }
                }
            } else {
                code_out = code_out.with_segment(segment);
            }
        }
        Ok(code_out)
    }

    fn dependency_lib_names(&self) -> Result<Vec<String>> {
        use crate::cargo_metadata;
        cargo_metadata::get_library_names(&self.config)
    }

    fn record_new_locals(
        &mut self,
        pat: ast::Pat,
        opt_ty: Option<ast::Type>,
        segment: &Segment,
        let_stmt_range: TextRange,
    ) {
        match pat {
            ast::Pat::IdentPat(ident) => self.record_local(ident, opt_ty, segment, let_stmt_range),
            ast::Pat::RecordPat(ref pat_struct) => {
                if let Some(record_fields) = pat_struct.record_pat_field_list() {
                    for field in record_fields.fields() {
                        if let Some(pat) = field.pat() {
                            self.record_new_locals(pat, None, segment, let_stmt_range);
                        }
                    }
                }
            }
            ast::Pat::TuplePat(ref pat_tuple) => {
                for pat in pat_tuple.fields() {
                    self.record_new_locals(pat, None, segment, let_stmt_range);
                }
            }
            ast::Pat::TupleStructPat(ref pat_tuple) => {
                for pat in pat_tuple.fields() {
                    self.record_new_locals(pat, None, segment, let_stmt_range);
                }
            }
            _ => {}
        }
    }

    fn record_local(
        &mut self,
        pat_ident: ast::IdentPat,
        opt_ty: Option<ast::Type>,
        segment: &Segment,
        let_stmt_range: TextRange,
    ) {
        // Default new variables to some type, say String. Assuming it isn't a
        // String, we'll get a compilation error when we try to move the
        // variable into our variable store, then we'll see what type the error
        // message says and fix it up. Hacky huh? If the user gave an explicit
        // type, we'll use that for all variables in that assignment (probably
        // only correct if it's a single variable). This gives the user a way to
        // force the type if rustc is giving us a bad suggestion.
        let type_name = match opt_ty {
            Some(ty) if type_is_fully_specified(&ty) => format!("{}", AstNode::syntax(&ty).text()),
            _ => "String".to_owned(),
        };
        if let Some(name) = ast::NameOwner::name(&pat_ident) {
            self.variable_states.insert(
                name.text().to_string(),
                VariableState {
                    type_name,
                    is_mut: pat_ident.mut_token().is_some(),
                    // All new locals will initially be defined only inside our catch_unwind
                    // block.
                    move_state: VariableMoveState::MovedIntoCatchUnwind,
                    // If we're preserving copy types, then assume this variable
                    // is copy until we find out it's not.
                    is_copy_type: self.config.preserve_vars_on_panic,
                    definition_span: segment.sequence.map(|segment_index| {
                        let range = name.syntax().text_range() - let_stmt_range.start();
                        UserCodeSpan {
                            segment_index,
                            range,
                        }
                    }),
                },
            );
        }
    }
}

fn replace_reserved_words_in_type(ty: &str) -> String {
    lazy_static! {
        static ref RESERVED_WORDS: Regex = Regex::new("(^|:|<)(async|try)(>|$|:)").unwrap();
    }
    RESERVED_WORDS.replace_all(ty, "${1}r#${2}${3}").to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_replace_reserved_words_in_type() {
        use super::replace_reserved_words_in_type as repl;
        assert_eq!(repl("asyncstart"), "asyncstart");
        assert_eq!(repl("endasync"), "endasync");
        assert_eq!(repl("async::foo"), "r#async::foo");
        assert_eq!(repl("foo::async::bar"), "foo::r#async::bar");
        assert_eq!(repl("foo::async::async::bar"), "foo::r#async::r#async::bar");
        assert_eq!(repl("Bar<async::foo::Baz>"), "Bar<r#async::foo::Baz>");
    }
}
