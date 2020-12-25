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

use crate::child_process::ChildProcess;
use crate::code_block::{CodeBlock, CodeKind};
use crate::crate_config::ExternalCrate;
use crate::errors::{bail, CompilationError, Error};
use crate::evcxr_internal_runtime;
use crate::item;
use crate::module::{Module, SoFile};
use crate::runtime;
use crate::rust_analyzer::{Completions, RustAnalyzer, VariableInfo};
use anyhow::Result;
use ra_ap_syntax::{ast, AstNode, SyntaxKind, SyntaxNode};
use regex::Regex;
use std;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tempfile;

pub struct EvalContext {
    // Our tmpdir if EVCXR_TMPDIR wasn't set - Drop causes tmpdir to be cleaned up.
    _tmpdir: Option<tempfile::TempDir>,
    build_num: i32,
    pub(crate) debug_mode: bool,
    opt_level: String,
    output_format: String,
    module: Module,
    state: ContextState,
    committed_state: ContextState,
    child_process: ChildProcess,
    // Whether we should preserve variables that are Copy when a panic occurs.
    // Sounds good, but unfortunately doing so currently requires an extra build
    // attempt to determine if the type of the variable is copy.
    pub preserve_vars_on_panic: bool,
    stdout_sender: mpsc::Sender<String>,
    stored_variable_states: HashMap<String, VariableState>,
    error_fmt: &'static ErrorFormat,
    analyzer: RustAnalyzer,
}

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
        EvalContext::with_subprocess_command(std::process::Command::new(&testing_runtime_path))
            .unwrap()
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
        let mut context = EvalContext {
            _tmpdir: opt_tmpdir,
            build_num: 0,
            debug_mode: false,
            opt_level: "2".to_owned(),
            output_format: "{:?}".to_owned(),
            state: ContextState::default(),
            committed_state: ContextState::default(),
            module,
            child_process,
            preserve_vars_on_panic: false,
            stdout_sender,
            stored_variable_states: HashMap::new(),
            error_fmt: &ERROR_FORMATS[0],
            analyzer,
        };
        let outputs = EvalContextOutputs {
            stdout: stdout_receiver,
            stderr: stderr_receiver,
        };
        if context.linker() == "lld" && context.eval("42").is_err() {
            context.set_linker("system".to_owned());
        } else {
            // We need to eval something anyway, otherwise rust-analyzer crashes when trying to get
            // completions. Not 100% sure. Just writing Cargo.toml isn't sufficient.
            context.eval("42")?;
        }
        Ok((context, outputs))
    }

    /// Evaluates the supplied Rust code.
    pub fn eval(&mut self, code: &str) -> Result<EvalOutputs, Error> {
        let (user_code, nodes) = CodeBlock::from_original_user_code(code);
        self.eval_with_callbacks(user_code, &nodes, &mut EvalCallbacks::default())
    }

    /// Evaluates the supplied Rust code.
    pub(crate) fn eval_with_callbacks(
        &mut self,
        user_code: CodeBlock,
        nodes: &[SyntaxNode],
        callbacks: &mut EvalCallbacks,
    ) -> Result<EvalOutputs, Error> {
        let mut phases = PhaseDetailsBuilder::new();

        if self.preserve_vars_on_panic {
            // Any pre-existing, non-copy variables are marked as available, so that we'll take their
            // values from outside of the catch_unwind block. If they remain this way, then this
            // effectively means that they're not being used.
            for variable_state in self.state.variable_states.values_mut() {
                variable_state.move_state = if variable_state.is_copy_type {
                    VariableMoveState::CopiedIntoCatchUnwind
                } else {
                    VariableMoveState::Available
                };
            }
        } else {
            for variable_state in self.state.variable_states.values_mut() {
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
                    self.record_new_locals(pat, let_stmt.ty());
                    code_out = code_out.with_segment(segment);
                }
            } else if ast::Expr::can_cast(node.kind()) {
                if statement_index == num_statements - 1 {
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
                                self.output_format
                            ))
                            .with_segment(segment)
                            .generated("));"),
                    );
                } else {
                    // We got an expression, but it wasn't the last
                    // statement, so don't try to print it.
                    code_out = code_out.with_segment(segment);
                }
            } else if let Some(item) = ast::Item::cast(node.clone()) {
                match item {
                    ast::Item::ExternCrate(extern_crate) => {
                        if let Some(crate_name) = extern_crate.name_ref() {
                            let crate_name = crate_name.text().to_string();
                            if !self.dependency_lib_names()?.contains(&crate_name) {
                                self.state
                                    .external_deps
                                    .entry(crate_name.clone())
                                    .or_insert_with(|| {
                                        ExternalCrate::new(crate_name.clone(), "\"*\"".to_owned())
                                            .unwrap()
                                    });
                            }
                            self.state
                                .extern_crate_stmts
                                .insert(crate_name, segment.code.clone());
                        }
                    }
                    ast::Item::MacroRules(macro_rules) => {
                        if let Some(name) = ast::NameOwner::name(&macro_rules) {
                            let item_block = CodeBlock::new().with_segment(segment);
                            self.state
                                .items_by_name
                                .insert(name.text().to_string(), item_block);
                        } else {
                            code_out = code_out.with_segment(segment);
                        }
                    }
                    ast::Item::Use(..) => {
                        self.state.use_stmts.insert(segment.code.clone());
                    }
                    item => {
                        let item_block = CodeBlock::new().with_segment(segment);
                        if let Some(item_name) = item::item_name(&item) {
                            *self
                                .state
                                .items_by_name
                                .entry(item_name.to_owned())
                                .or_default() = item_block;
                            previous_item_name = Some(item_name);
                        } else if let Some(item_name) = &previous_item_name {
                            // unwrap below should never fail because we put
                            // that key in the map on a previous iteration,
                            // otherwise we wouldn't have had a value in
                            // `previous_item_name`.
                            self.state
                                .items_by_name
                                .get_mut(item_name)
                                .unwrap()
                                .modify(move |block_for_name| block_for_name.add_all(item_block));
                        } else {
                            self.state.unnamed_items.push(item_block);
                        }
                    }
                }
            } else {
                code_out = code_out.with_segment(segment);
            }
        }

        let mut outputs = match self.run_statements(code_out, &mut phases, callbacks) {
            Err(error) => {
                if let Error::ChildProcessTerminated(_) = error {
                    self.restart_child_process()?;
                    self.commit_state();
                } else {
                    self.state = self.committed_state.clone();
                }
                return Err(error.without_non_reportable_errors());
            }
            Ok(x) => x,
        };

        // Once, we reach here, our code has successfully executed, so we
        // conclude that variable changes are now applied.
        self.commit_state();

        phases.phase_complete("Execution");
        outputs.phases = phases.phases;

        Ok(outputs)
    }

    pub(crate) fn completions(
        &mut self,
        user_code: CodeBlock,
        offset: usize,
    ) -> Result<Completions> {
        let code = self.code_to_compile(user_code, CompilationMode::NoCatch);
        let wrapped_offset = code.user_offset_to_output_offset(offset)?;
        self.analyzer.set_source(code.code_string())?;
        let mut completions = self.analyzer.completions(wrapped_offset)?;
        completions.start_offset = code.output_offset_to_user_offset(completions.start_offset)?;
        completions.end_offset = code.output_offset_to_user_offset(completions.end_offset)?;
        Ok(completions)
    }

    pub fn last_source(&self) -> Result<String, std::io::Error> {
        self.module.last_source()
    }

    pub fn time_passes(&self) -> bool {
        self.module.time_passes
    }

    pub fn set_time_passes(&mut self, value: bool) {
        self.module.time_passes = value;
    }

    pub fn opt_level(&self) -> &str {
        &self.opt_level
    }

    pub fn set_opt_level(&mut self, level: &str) -> Result<(), Error> {
        if level.is_empty() {
            bail!("Optimization level cannot be an empty string");
        }
        self.opt_level = level.to_owned();
        Ok(())
    }
    pub fn output_format(&self) -> &str {
        &self.output_format
    }

    pub fn set_output_format(&mut self, output_format: String) {
        self.output_format = output_format;
    }

    pub fn set_sccache(&mut self, enabled: bool) -> Result<(), Error> {
        self.module.set_sccache(enabled)
    }

    pub fn sccache(&self) -> bool {
        self.module.sccache()
    }

    pub fn set_error_format(&mut self, format_str: &str) -> Result<(), Error> {
        for format in ERROR_FORMATS {
            if format.format_str == format_str {
                self.error_fmt = format;
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
        self.error_fmt.format_str
    }

    pub fn error_format_trait(&self) -> &str {
        self.error_fmt.format_trait
    }

    pub fn set_linker(&mut self, linker: String) {
        self.module.linker = linker;
    }

    pub fn linker(&self) -> &str {
        &self.module.linker
    }

    // TODO: Remove this function and just use add_dep().
    pub fn add_extern_crate(&mut self, name: String, config: String) -> Result<EvalOutputs, Error> {
        self.add_dep(&name, &config)?;
        let result = self.eval("");
        if result.is_err() {
            self.state.external_deps.remove(&name);
        }
        result
    }

    /// Adds a crate dependency with the specified name and configuration.
    /// Actual compilation is deferred until the next call to eval. If that call
    /// fails, then this dependency will be reverted. If you want to compile
    /// straight away and ensure that the change is committed, then follow this
    /// call with a call to eval("");
    pub fn add_dep(&mut self, name: &str, config: &str) -> Result<(), Error> {
        self.state.external_deps.insert(
            name.to_owned(),
            ExternalCrate::new(name.to_owned(), config.to_owned())?,
        );
        Ok(())
    }

    pub fn debug_mode(&self) -> bool {
        self.debug_mode
    }

    pub fn set_debug_mode(&mut self, debug_mode: bool) {
        self.debug_mode = debug_mode;
    }

    pub fn variables_and_types(&self) -> impl Iterator<Item = (&str, &str)> {
        self.state
            .variable_states
            .iter()
            .map(|(v, t)| (v.as_str(), t.type_name.as_str()))
    }

    pub fn defined_item_names(&self) -> impl Iterator<Item = &str> {
        self.state.items_by_name.keys().map(String::as_str)
    }

    // Clears all state, while keeping tmpdir. This allows us to effectively
    // restart, but without having to recompile any external crates we'd already
    // compiled.
    pub fn clear(&mut self) -> Result<(), Error> {
        self.state = ContextState::default();
        self.stored_variable_states = HashMap::new();
        self.restart_child_process()
    }

    fn restart_child_process(&mut self) -> Result<(), Error> {
        self.state.variable_states.clear();
        self.stored_variable_states = HashMap::new();
        self.child_process = self.child_process.restart()?;
        Ok(())
    }

    pub(crate) fn format_cargo_deps(&self) -> String {
        self.state
            .external_deps
            .values()
            .map(|krate| format!("{} = {}\n", krate.name, krate.config))
            .collect::<Vec<_>>()
            .join("")
    }

    pub(crate) fn last_compile_dir(&self) -> &Path {
        self.module.crate_dir()
    }

    fn commit_state(&mut self) {
        self.stored_variable_states = self.state.variable_states.clone();
        self.committed_state = self.state.clone();
    }

    fn dependency_lib_names(&self) -> Result<Vec<String>> {
        use crate::cargo_metadata;
        cargo_metadata::get_library_names(self.module.crate_dir())
    }

    fn compilation_mode(&self) -> CompilationMode {
        if self.preserve_vars_on_panic {
            CompilationMode::RunAndCatchPanics
        } else {
            CompilationMode::NoCatch
        }
    }

    fn run_statements(
        &mut self,
        mut user_code: CodeBlock,
        phases: &mut PhaseDetailsBuilder,
        callbacks: &mut EvalCallbacks,
    ) -> Result<EvalOutputs, Error> {
        self.write_cargo_toml()?;
        self.fix_variable_types(self.code_to_compile(user_code.clone(), self.compilation_mode()))?;
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
                self.compilation_mode(),
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
                            self.attempt_to_fix_error(error, &mut user_code, &mut fixed)?;
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
                            CompilationMode::NoCatchExpectError,
                            phases,
                            callbacks,
                        )?;
                    }
                    return Err(Error::CompilationErrors(errors));
                }

                Err(Error::TypeRedefinedVariablesLost(variables)) => {
                    for variable in &variables {
                        self.state.variable_states.remove(variable);
                        self.stored_variable_states.remove(variable);
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
        compilation_mode: CompilationMode,
        phases: &mut PhaseDetailsBuilder,
        callbacks: &mut EvalCallbacks,
    ) -> Result<ExecutionArtifacts, Error> {
        let code = self.code_to_compile(user_code, compilation_mode);
        let so_file = self.module.compile(&code)?;

        if compilation_mode == CompilationMode::NoCatchExpectError {
            // Uh-oh, caller was expecting an error, return OK and the caller can return the
            // original error.
            return Ok(ExecutionArtifacts {
                output: EvalOutputs::new(),
            });
        }
        phases.phase_complete("Final compile");

        let output = self.run_and_capture_output(&so_file, callbacks)?;
        Ok(ExecutionArtifacts { output })
    }

    pub(crate) fn write_cargo_toml(&self) -> Result<()> {
        self.module.write_cargo_toml(self)?;
        Ok(())
    }

    fn fix_variable_types(&mut self, code: CodeBlock) -> Result<(), Error> {
        self.analyzer.set_source(code.code_string())?;
        for (
            variable_name,
            VariableInfo {
                type_name,
                is_mutable,
            },
        ) in self
            .analyzer
            .top_level_variables(&self.current_user_fn_name())
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
            let preserve_vars_on_panic = self.preserve_vars_on_panic;
            self.state
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
                })
                .type_name = type_name;
        }
        Ok(())
    }

    fn code_to_compile(
        &self,
        user_code: CodeBlock,
        compilation_mode: CompilationMode,
    ) -> CodeBlock {
        let mut code = CodeBlock::new()
            .generated("#![allow(unused_imports, unused_mut, dead_code)]")
            .add_all(self.get_imports());
        for item in self
            .state
            .items_by_name
            .values()
            .chain(self.state.unnamed_items.iter())
        {
            code = code.add_all(item.clone());
        }
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

    fn wrap_user_code(
        &self,
        mut user_code: CodeBlock,
        compilation_mode: CompilationMode,
    ) -> CodeBlock {
        let needs_variable_store = !self.state.variable_states.is_empty()
            || !self.stored_variable_states.is_empty()
            || self.state.async_mode
            || self.state.allow_question_mark;
        let mut code = CodeBlock::new();
        if self.state.allow_question_mark {
            code = code.generated(format!(
                r#"
                struct EvcxrUserCodeError {{}}
                impl<T: {}> From<T> for EvcxrUserCodeError {{
                    fn from(error: T) -> Self {{
                        eprintln!("{}", error);
                        println!("{{}}", evcxr_internal_runtime::USER_ERROR_OCCURRED);
                        EvcxrUserCodeError {{}}
                    }}
                }}
            "#,
                self.error_fmt.format_trait, self.error_fmt.format_str
            ));
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
        if self.state.async_mode {
            user_code = CodeBlock::new()
                .generated(stringify!(evcxr_variable_store
                    .lazy_arc("evcxr_tokio_runtime", || std::sync::Mutex::new(
                        tokio::runtime::Runtime::new().unwrap()
                    ))
                    .lock()
                    .unwrap()))
                .generated(".block_on(async {")
                .add_all(user_code);
            if self.state.allow_question_mark {
                user_code = CodeBlock::new()
                    .generated("let _ =")
                    .add_all(user_code)
                    .generated("Ok::<(), EvcxrUserCodeError>(())");
            }
            user_code = user_code.generated("});")
        } else if self.state.allow_question_mark {
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

    fn current_user_fn_name(&self) -> String {
        format!("run_user_code_{}", self.build_num)
    }

    fn run_and_capture_output(
        &mut self,
        so_file: &SoFile,
        callbacks: &mut EvalCallbacks,
    ) -> Result<EvalOutputs, Error> {
        let mut output = EvalOutputs::new();
        // TODO: We should probably send an OsString not a String. Otherwise
        // things won't work if the path isn't UTF-8 - apparently that's a thing
        // on some platforms.
        let fn_name = self.current_user_fn_name();
        self.child_process.send(&format!(
            "LOAD_AND_RUN {} {}",
            so_file.path.to_string_lossy(),
            fn_name,
        ))?;

        self.build_num += 1;

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
                self.state
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
            self.state
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
                            self.state
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
                            &mut self
                                .state
                                .variable_states
                                .get_mut(variable_name)
                                .unwrap()
                                .move_state,
                            VariableMoveState::MovedIntoCatchUnwind,
                        );
                        if old_move_state == VariableMoveState::MovedIntoCatchUnwind {
                            // Variable is truly moved, forget about it.
                            self.state.variable_states.remove(variable_name);
                        }
                        fixed_errors.insert("Captured value");
                    } else if error.code() == Some("E0425") {
                        // cannot find value in scope.
                        self.state.variable_states.remove(variable_name);
                        fixed_errors.insert("Variable moved");
                    } else if error.code() == Some("E0603") {
                        if let Some(variable_state) =
                            self.state.variable_states.remove(variable_name)
                        {
                            bail!(
                                "Failed to determine type of variable `{}`. rustc suggested type \
                             {}, but that's private. Sometimes adding an extern crate will help \
                             rustc suggest the correct public type name, or you can give an \
                             explicit type.",
                                variable_name,
                                variable_state.type_name
                            );
                        }
                    } else if error.code() == Some("E0562") || error.code().is_none() {
                        bail!(
                            "The variable `{}` has a type `{}` that can't be persisted. You can \
                            try wrapping your code in braces so that the variable goes out of \
                            scope before the end of the code to be executed.",
                            variable_name,
                            self.state.variable_states[variable_name].type_name
                        );
                    }
                }
                CodeKind::AssertCopyType { variable_name } => {
                    if error.code() == Some("E0277") {
                        if let Some(state) = self.state.variable_states.get_mut(variable_name) {
                            state.is_copy_type = false;
                            fixed_errors.insert("Non-copy type");
                        }
                    }
                }
                CodeKind::WithFallback(fallback) => {
                    user_code.apply_fallback(fallback);
                    fixed_errors.insert("Fallback");
                }
                CodeKind::OriginalUserCode(_) | CodeKind::OtherUserCode => {
                    if error.code() == Some("E0728") && !self.state.async_mode {
                        self.state.async_mode = true;
                        if !self.state.external_deps.contains_key("tokio") {
                            self.add_dep("tokio", "\"0.2\"")?;
                        }
                        fixed_errors.insert("Enabled async mode");
                    } else if error.code() == Some("E0277") && !self.state.allow_question_mark {
                        self.state.allow_question_mark = true;
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

    fn record_new_locals(&mut self, pat: ast::Pat, opt_ty: Option<ast::Type>) {
        match pat {
            ast::Pat::IdentPat(ident) => self.record_local(ident, opt_ty),
            ast::Pat::RecordPat(ref pat_struct) => {
                if let Some(record_fields) = pat_struct.record_pat_field_list() {
                    for field in record_fields.fields() {
                        if let Some(pat) = field.pat() {
                            self.record_new_locals(pat, None);
                        }
                    }
                }
            }
            ast::Pat::TuplePat(ref pat_tuple) => {
                for pat in pat_tuple.fields() {
                    self.record_new_locals(pat, None);
                }
            }
            ast::Pat::TupleStructPat(ref pat_tuple) => {
                for pat in pat_tuple.fields() {
                    self.record_new_locals(pat, None);
                }
            }
            _ => {}
        }
    }

    fn record_local(&mut self, pat_ident: ast::IdentPat, opt_ty: Option<ast::Type>) {
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
            self.state.variable_states.insert(
                name.text().to_string(),
                VariableState {
                    type_name,
                    is_mut: pat_ident.mut_token().is_some(),
                    // All new locals will initially be defined only inside our catch_unwind
                    // block.
                    move_state: VariableMoveState::MovedIntoCatchUnwind,
                    // If we're preserving copy types, then assume this variable
                    // is copy until we find out it's not.
                    is_copy_type: self.preserve_vars_on_panic,
                },
            );
        }
    }

    fn store_variable_statements(&self, move_state: &VariableMoveState) -> CodeBlock {
        let mut statements = CodeBlock::new();
        for (var_name, var_state) in &self.state.variable_states {
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

    fn get_imports(&self) -> CodeBlock {
        let mut extern_stmts = CodeBlock::new();
        let mut use_stmts = CodeBlock::new();
        for stmt in self.state.extern_crate_stmts.values() {
            extern_stmts = extern_stmts.other_user_code(stmt.clone());
        }
        for user_use_stmt in &self.state.use_stmts {
            use_stmts = use_stmts.other_user_code(user_use_stmt.clone());
        }
        extern_stmts.add_all(use_stmts)
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

#[derive(Clone)]
struct VariableState {
    type_name: String,
    is_mut: bool,
    move_state: VariableMoveState,
    // Whether the type of this variable implements Copy. Variables that implement copy never get
    // moved into the catch_unwind block (they get copied), so we need to make sure we always save
    // them from within the catch_unwind block, otherwise any changes made to the variable within
    // the block will be lost.
    is_copy_type: bool,
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
#[derive(Clone)]
struct ContextState {
    items_by_name: HashMap<String, CodeBlock>,
    unnamed_items: Vec<CodeBlock>,
    pub(crate) external_deps: HashMap<String, ExternalCrate>,
    use_stmts: HashSet<String>,
    // Keyed by crate name. Could use a set, except that the statement might be
    // formatted slightly differently.
    extern_crate_stmts: HashMap<String, String>,
    variable_states: HashMap<String, VariableState>,
    async_mode: bool,
    allow_question_mark: bool,
}

impl ContextState {
    fn default() -> ContextState {
        ContextState {
            items_by_name: HashMap::new(),
            unnamed_items: vec![],
            external_deps: HashMap::new(),
            use_stmts: HashSet::new(),
            extern_crate_stmts: HashMap::new(),
            variable_states: HashMap::new(),
            async_mode: false,
            allow_question_mark: false,
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
