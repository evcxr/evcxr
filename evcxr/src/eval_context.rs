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

use child_process::ChildProcess;
use code_block::{CodeBlock, CodeOrigin};
use crate_config::ExternalCrate;
use errors::{CompilationError, Error};
use evcxr_internal_runtime;
use idents;
use item;
use module::Module;
use rand;
use regex::Regex;
use runtime;
use statement_splitter;
use std;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use syn;
use tempfile;

pub struct EvalContext {
    // Our tmpdir if EVCXR_TMPDIR wasn't set - Drop causes tmpdir to be cleaned up.
    _tmpdir: Option<tempfile::TempDir>,
    pub(crate) tmpdir_path: PathBuf,
    crate_suffix: String,
    variable_states: HashMap<String, VariableState>,
    build_num: i32,
    load_variable_statements: CodeBlock,
    pub(crate) debug_mode: bool,
    opt_level: String,
    next_module: Arc<Mutex<Option<Module>>>,
    state: ContextState,
    child_process: ChildProcess,
    // Whether we'll pre-warm each compiled crate by compiling the same code as
    // was in the previous crate, but with the new crate name.
    pub should_pre_warm: bool,
    stdout_sender: mpsc::Sender<String>,
}

// Outputs from an EvalContext. This is a separate struct since users may want
// destructure this and pass its components to separate threads.
pub struct EvalContextOutputs {
    pub stdout: mpsc::Receiver<String>,
    pub stderr: mpsc::Receiver<String>,
}

impl Drop for EvalContext {
    fn drop(&mut self) {
        // Make sure any warming-up has finished.
        self.take_next_module();
        // Probably doesn't matter much (since dlclose will just decrement refcount), but seems like
        // we should unload modules in reverse order.
        while self.state.modules.pop().is_some() {}
    }
}

fn target_dir(tmpdir: &Path) -> PathBuf {
    tmpdir.join("target")
}

fn deps_dir(tmpdir: &Path) -> PathBuf {
    target_dir(tmpdir).join("debug").join("deps")
}

impl EvalContext {
    pub fn new() -> Result<(EvalContext, EvalContextOutputs), Error> {
        let current_exe = std::env::current_exe()?;
        Self::with_subprocess_command(std::process::Command::new(&current_exe))
    }

    #[cfg(window)]
    fn apply_platform_specific_vars(tmpdir_path: &Path, command: &mut std::process::Command) {
        // Windows doesn't support rpath, so we need to set PATH so that it
        // knows where to find dlls.
        use std::ffi::OsString;
        let mut path_var_value = OsString::new();
        path_var_value.push(&deps_dir(tmpdir_path));
        path_var_value.push(";");
        path_var_value.push(std::env::var("PATH").unwrap_or_default());
        command.env("PATH", path_var_value);
    }

    #[cfg(not(window))]
    fn apply_platform_specific_vars(_tmpdir_path: &Path, _command: &mut std::process::Command) {}

    pub fn with_subprocess_command(
        mut subprocess_command: std::process::Command,
    ) -> Result<(EvalContext, EvalContextOutputs), Error> {
        let mut opt_tmpdir = None;
        let tmpdir_path;
        let crate_suffix;
        if let Ok(from_env) = std::env::var("EVCXR_TMPDIR") {
            tmpdir_path = PathBuf::from(from_env);
            // If we've specified a tmpdir, there may be multiple contexts
            // sharing it, so add a suffix to our crate names.
            crate_suffix = format!("{:x}_", rand::random::<u32>());
        } else {
            let tmpdir = tempfile::tempdir()?;
            tmpdir_path = PathBuf::from(tmpdir.path());
            opt_tmpdir = Some(tmpdir);
            crate_suffix = String::new();
        }

        Self::apply_platform_specific_vars(&tmpdir_path, &mut subprocess_command);

        let (stdout_sender, stdout_receiver) = mpsc::channel();
        let (stderr_sender, stderr_receiver) = mpsc::channel();
        let child_process = ChildProcess::new(subprocess_command, stderr_sender)?;
        let mut context = EvalContext {
            _tmpdir: opt_tmpdir,
            tmpdir_path,
            crate_suffix,
            variable_states: HashMap::new(),
            build_num: 0,
            load_variable_statements: CodeBlock::new(),
            debug_mode: false,
            opt_level: "2".to_owned(),
            state: ContextState::default(),
            next_module: Arc::new(Mutex::new(None)),
            child_process,
            should_pre_warm: true,
            stdout_sender,
        };
        context.add_internal_runtime()?;
        let outputs = EvalContextOutputs {
            stdout: stdout_receiver,
            stderr: stderr_receiver,
        };
        Ok((context, outputs))
    }

    pub(crate) fn target_dir(&self) -> PathBuf {
        target_dir(&self.tmpdir_path)
    }

    pub(crate) fn deps_dir(&self) -> PathBuf {
        deps_dir(&self.tmpdir_path)
    }

    /// Evaluates the supplied Rust code.
    pub fn eval(&mut self, code: &str) -> Result<EvalOutputs, Error> {
        fn parse_stmt_or_expr(code: &str) -> Result<syn::Stmt, ()> {
            match syn::parse_str::<syn::Stmt>(code) {
                Ok(stmt) => Ok(stmt),
                Err(_) => match syn::parse_str::<syn::Expr>(code) {
                    Ok(expr) => Ok(syn::Stmt::Expr(expr)),
                    Err(_) => Err(()),
                },
            }
        }

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

        // Copy our state, so that changes we make to it can be rolled back if compilation fails.
        let old_state = self.state.clone();

        let mut top_level_items = CodeBlock::new();
        let mut code_block = CodeBlock::new();
        let mut defined_names = Vec::new();
        for stmt_code in statement_splitter::split_into_statements(code) {
            if let Ok(stmt) = parse_stmt_or_expr(stmt_code) {
                if self.debug_mode {
                    println!("STMT: {:#?}", stmt);
                }
                match &stmt {
                    syn::Stmt::Local(local) => {
                        for pat in &local.pats {
                            self.record_new_locals(pat, local.ty.as_ref().map(|ty| &*ty.1));
                        }
                        code_block = code_block.user_code(stmt_code);
                    }
                    syn::Stmt::Item(syn::Item::ExternCrate(syn::ItemExternCrate {
                        ident, ..
                    })) => {
                        let crate_name = ident.to_string();
                        if !self.dependency_lib_names()?.contains(&crate_name) {
                            self.state
                                .external_deps
                                .entry(crate_name.clone())
                                .or_insert_with(|| {
                                    ExternalCrate::new(crate_name, "\"*\"".to_owned()).unwrap()
                                });
                        }
                        self.state.extern_crate_stmts.insert(stmt_code.to_owned());
                    }
                    syn::Stmt::Item(syn::Item::Macro(_)) | syn::Stmt::Semi(..) => {
                        code_block = code_block.user_code(stmt_code);
                    }
                    syn::Stmt::Item(syn::Item::Use(..)) => {
                        self.state.use_stmts.insert(stmt_code.to_owned());
                    }
                    syn::Stmt::Item(item) => {
                        if !item::is_item_public(item) {
                            bail!(
                                "Items currently need to be explicitly made pub along \
                                 with all fields of structs."
                            );
                        }
                        if let Some(item_name) = item::item_name(item) {
                            defined_names.push(item_name.to_owned());
                        }
                        top_level_items = top_level_items.user_code(stmt_code);
                    }
                    syn::Stmt::Expr(_) => {
                        code_block = code_block.code_with_fallback(
                            // First we try calling .evcxr_display().
                            CodeBlock::new()
                                .generated("(")
                                .user_code(stmt_code)
                                .generated(").evcxr_display();")
                                .to_string(),
                            // If that fails, we try debug format.
                            CodeBlock::new()
                                .generated(
                                    "evcxr_internal_runtime::send_text_plain(&format!(\"{:?}\",\n",
                                ).user_code(stmt_code)
                                .generated("));"),
                        );
                    }
                }
            } else {
                // Syn couldn't parse the code, put it inside a function body and hopefully we'll
                // get a reasonable error message from rustc.
                code_block = code_block.user_code(stmt_code);
            }
        }

        // Find any modules that previously defined the names defined by our new module and prevent
        // those old definitions from being imported in future.
        for name in &defined_names {
            for module in &mut self.state.modules {
                module.defined_names.retain(|n| n != name);
            }
        }

        let outputs = match self.compile_items_then_run_statements(
            code_block,
            &top_level_items,
            defined_names,
        ) {
            Err(error) => {
                if let Error::ChildProcessTerminated(_) = error {
                    self.restart_child_process()?;
                }
                self.state = old_state;
                return Err(error.without_non_reportable_errors());
            }
            Ok(x) => x,
        };

        if self.should_pre_warm {
            if let Some(last_module) = self.state.modules.pop() {
                self.warm_up_next_module(&CodeBlock::new(), &last_module.module)?;
                self.state.modules.push(last_module);
            }
        }

        // Our load_variable_statements are only updated if we successfully run
        // the code, which if we get here has happened.
        self.load_variable_statements = self.load_variable_statements();

        Ok(outputs)
    }

    pub fn opt_level(&self) -> &str {
        &self.opt_level
    }

    pub fn set_opt_level(&mut self, level: &str) -> Result<(), Error> {
        if self.build_num > 0 {
            bail!("Optimization level cannot be set after code has been executed.");
        }
        if level.is_empty() {
            bail!("Optimization level cannot be an empty string");
        }
        self.opt_level = level.to_owned();
        Ok(())
    }

    pub fn add_extern_crate(&mut self, name: String, config: String) -> Result<(), Error> {
        self.state
            .external_deps
            .insert(name.clone(), ExternalCrate::new(name, config)?);
        self.eval("").map(|_| ())
    }

    pub fn debug_mode(&self) -> bool {
        self.debug_mode
    }

    pub fn set_debug_mode(&mut self, debug_mode: bool) {
        self.debug_mode = debug_mode;
    }

    pub fn variables_and_types(&self) -> impl Iterator<Item = (&str, &str)> {
        self.variable_states
            .iter()
            .map(|(v, t)| (v.as_str(), t.type_name.as_str()))
    }

    pub fn defined_item_names(&self) -> impl Iterator<Item = &str> {
        struct It<'a> {
            module_iter: std::slice::Iter<'a, ModuleState>,
            name_iter: Option<std::slice::Iter<'a, String>>,
        }

        impl<'a> Iterator for It<'a> {
            type Item = &'a str;

            fn next(&mut self) -> Option<&'a str> {
                loop {
                    if let Some(name_iter) = &mut self.name_iter {
                        if let Some(name) = name_iter.next() {
                            return Some(name);
                        }
                    }
                    if let Some(module) = self.module_iter.next() {
                        self.name_iter = Some(module.defined_names.iter());
                    } else {
                        return None;
                    }
                }
            }
        }

        It {
            module_iter: self.state.modules.iter(),
            name_iter: None,
        }
    }

    // Clears all state, while keeping tmpdir. This allows us to effectively
    // restart, but without having to recompile any external crates we'd already
    // compiled.
    pub fn clear(&mut self) -> Result<(), Error> {
        self.state = ContextState::default();
        self.add_internal_runtime()?;
        self.restart_child_process()
    }

    fn restart_child_process(&mut self) -> Result<(), Error> {
        self.variable_states.clear();
        self.load_variable_statements = CodeBlock::new();
        self.child_process = self.child_process.restart()?;
        Ok(())
    }

    fn add_internal_runtime(&mut self) -> Result<(), Error> {
        let mut runtime_module = Module::new(self, "evcxr_internal_runtime", None)?;
        runtime_module.write_sources_and_compile(
            self,
            &CodeBlock::new().generated(include_str!("evcxr_internal_runtime.rs")),
        )?;
        self.state
            .modules
            .push(ModuleState::new(runtime_module, Vec::new()));
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

    pub(crate) fn last_compile_dir(&self) -> &Option<PathBuf> {
        &self.state.last_compile_dir
    }

    fn dependency_lib_names(&self) -> Result<Vec<String>, Error> {
        use cargo_metadata;
        if let Some(dir) = self.last_compile_dir() {
            cargo_metadata::get_library_names(&dir)
        } else {
            Ok(vec![])
        }
    }

    // If we have just top-level items, compile them. If we have just user-code,
    // compile and run it. If we have both, then process them as separate
    // crates. It'd be nice if we didn't have to do this, but it's currently
    // necessary in order to ensure that any variables in the user code that
    // reference types in the top-level items end up with fully qualified types.
    // Having types that aren't fully qualified can become a problem if new
    // types with the same name are later defined, since then the variable
    // "changes type" from our perspective, which causes us to fail to retrieve
    // it from the Any.
    fn compile_items_then_run_statements(
        &mut self,
        user_code: CodeBlock,
        top_level_items: &CodeBlock,
        defined_names: Vec<String>,
    ) -> Result<EvalOutputs, Error> {
        if !top_level_items.is_empty() && !user_code.is_empty() {
            // Both
            self.run_statements(CodeBlock::new(), top_level_items, defined_names)?;
            return self.run_statements(user_code, &CodeBlock::new(), vec![]);
        }
        if !top_level_items.is_empty() {
            // Just items.
            return self.run_statements(CodeBlock::new(), top_level_items, defined_names);
        }
        // Either just code, or neither. In the case of neither, we still want
        // to make sure we try building, in case the user ran just a
        // use-statement by itself. Since they're stored separately, the user
        // code and top-level-items will both end up empty.
        self.run_statements(user_code, &CodeBlock::new(), vec![])
    }

    fn run_statements(
        &mut self,
        mut user_code: CodeBlock,
        top_level_items: &CodeBlock,
        defined_names: Vec<String>,
    ) -> Result<EvalOutputs, Error> {
        // In some circumstances we may need a few tries before we get the code right. Note that
        // we'll generally give up sooner than this if there's nothing left that we think we can
        // fix. The limit is really to prevent retrying indefinitely in case our "fixing" of things
        // somehow ends up flip-flopping back and forth. Not sure how that could happen, but best to
        // avoid any infinite loops.
        let mut remaining_retries = 5;
        loop {
            // Try to compile and run the code.
            let result = self.try_run_statements(
                user_code.clone(),
                top_level_items.clone(),
                CompilationMode::RunAndCatchPanics,
            );
            match result {
                Ok(execution_artifacts) => {
                    let module = execution_artifacts.module;
                    self.state.last_compile_dir = Some(module.crate_dir.clone());

                    if !defined_names.is_empty() {
                        self.state
                            .modules
                            .push(ModuleState::new(module, defined_names));
                    }
                    return Ok(execution_artifacts.output);
                }

                Err(Error::CompilationErrors(errors)) => {
                    // If we failed to compile, attempt to deal with the first
                    // round of compilation errors by adjusting variable types,
                    // whether they've been moved into the catch_unwind block
                    // etc.
                    if remaining_retries > 0 {
                        let mut retry = false;
                        for error in &errors {
                            retry |= self.attempt_to_fix_error(error, &mut user_code)?;
                        }
                        if retry {
                            remaining_retries -= 1;
                            continue;
                        }
                    }
                    if !user_code.is_empty() {
                        // We have user code and it appears to have an error, recompile without
                        // catch_unwind to try and get a better error message. e.g. we don't want the
                        // user to see messages like "cannot borrow immutable captured outer variable in
                        // an `FnOnce` closure `a` as mutable".
                        self.try_run_statements(
                            user_code.clone(),
                            top_level_items.clone(),
                            CompilationMode::NoCatchExpectError,
                        )?;
                    }
                    return Err(Error::CompilationErrors(errors));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn try_run_statements(
        &mut self,
        user_code: CodeBlock,
        top_level_items: CodeBlock,
        compilation_mode: CompilationMode,
    ) -> Result<ExecutionArtifacts, Error> {
        let mut module = match self.take_next_module() {
            Some(m) => m,
            None => self.create_new_module(None)?,
        };
        let mut code = CodeBlock::new()
            .generated("#![allow(unused_imports)]")
            .add_all(self.get_imports())
            .add_all(top_level_items);
        let has_user_code = !user_code.is_empty();
        if has_user_code {
            code = code.add_all(self.wrap_user_code(user_code, compilation_mode, &module));
        } else {
            // TODO: Add a mechanism to load a crate without any function to call then remove this.
            code = code
                .generated("#[no_mangle]")
                .generated(format!("pub extern \"C\" fn {}(", module.user_fn_name))
                .generated("mut x: *mut std::os::raw::c_void) -> *mut std::os::raw::c_void {x}");
        }
        if let Err(error) = module.write_sources_and_compile(self, &code) {
            // Compilation failed, reuse this module next time.
            *self.next_module.lock().unwrap() = Some(module);
            return Err(error);
        }
        if compilation_mode == CompilationMode::NoCatchExpectError {
            // Uh-oh, caller was expecting an error, return OK and the caller can return the
            // original error.
            return Ok(ExecutionArtifacts {
                output: EvalOutputs::new(),
                module,
            });
        }

        let output = self.run_and_capture_output(&module)?;
        Ok(ExecutionArtifacts { output, module })
    }

    fn wrap_user_code(
        &mut self,
        user_code: CodeBlock,
        compilation_mode: CompilationMode,
        module: &Module,
    ) -> CodeBlock {
        let mut code = CodeBlock::new()
            .generated("#[no_mangle]")
            .generated(format!("pub extern \"C\" fn {}(", module.user_fn_name))
            .generated("mut evcxr_variable_store: *mut evcxr_internal_runtime::VariableStore)")
            .generated("  -> *mut evcxr_internal_runtime::VariableStore {")
            .generated("if evcxr_variable_store.is_null() {")
            .generated("  evcxr_variable_store = evcxr_internal_runtime::create_variable_store();")
            .generated("}")
            .generated("let evcxr_variable_store = unsafe {&mut *evcxr_variable_store};")
            .add_all(self.load_variable_statements.clone());
        if compilation_mode == CompilationMode::RunAndCatchPanics {
            code = code
                .generated("match std::panic::catch_unwind(")
                .generated("  std::panic::AssertUnwindSafe(move ||{")
                // Shadow the outer evcxr_variable_store with a local one for variables moved
                // into the closure.
                .generated(
                    "let mut evcxr_variable_store = evcxr_internal_runtime::VariableStore::new();",
                ).add_all(user_code)
                .add_all(self.store_variable_statements(&VariableMoveState::MovedIntoCatchUnwind))
                .add_all(self.store_variable_statements(&VariableMoveState::CopiedIntoCatchUnwind))
                // Return our local variable store from the closure to be merged back into the
                // main variable store.
                .generated("evcxr_variable_store")
                .generated("})) { ")
                .generated("  Ok(inner_store) => evcxr_variable_store.merge(inner_store),")
                .generated("  Err(_) => {")
                .add_all(self.store_variable_statements(&VariableMoveState::CopiedIntoCatchUnwind))
                .generated("    evcxr_internal_runtime::notify_panic()}")
                .generated("}");
        } else {
            code = code.add_all(user_code);
        }
        code = code.add_all(self.store_variable_statements(&VariableMoveState::Available));
        if compilation_mode != CompilationMode::RunAndCatchPanics {
            code = code
                .add_all(self.store_variable_statements(&VariableMoveState::MovedIntoCatchUnwind))
                .add_all(self.store_variable_statements(&VariableMoveState::CopiedIntoCatchUnwind));
        }
        code = code.generated("evcxr_variable_store}");
        code
    }

    fn run_and_capture_output(&mut self, module: &Module) -> Result<EvalOutputs, Error> {
        let mut output = EvalOutputs::new();

        self.child_process.send(&format!(
            "LOAD_AND_RUN {} {}",
            module.so_path.to_str().unwrap(),
            module.user_fn_name,
        ))?;

        let mut got_panic = false;
        lazy_static! {
            static ref MIME_OUTPUT: Regex = Regex::new("EVCXR_BEGIN_CONTENT ([^ ]+)").unwrap();
        }
        loop {
            let line = self.child_process.recv_line()?;
            if line == runtime::EVCXR_EXECUTION_COMPLETE {
                break;
            }
            if line == evcxr_internal_runtime::PANIC_NOTIFICATION {
                got_panic = true;
            } else if let Some(captures) = MIME_OUTPUT.captures(&line) {
                let mime_type = captures[1].to_owned();
                let mut content = String::new();
                loop {
                    let line = self.child_process.recv_line()?;
                    if line == "EVCXR_END_CONTENT" {
                        break;
                    }
                    if line == evcxr_internal_runtime::PANIC_NOTIFICATION {
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
            self.variable_states
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
        };
        Ok(output)
    }

    fn attempt_to_fix_error(
        &mut self,
        error: &CompilationError,
        user_code: &mut CodeBlock,
    ) -> Result<bool, Error> {
        let mut retry = false;
        let error_code = match error.code() {
            Some(c) => c,
            _ => return Ok(false),
        };
        for code_origin in &error.code_origins {
            match code_origin {
                CodeOrigin::PackVariable { variable_name } => {
                    if error_code == "E0308" {
                        // mismatched types
                        if let Some(mut actual_type) = error.get_actual_type() {
                            // If the user hasn't given enough information for the compiler to
                            // determine what type of integer or float, we default to i32 and f32
                            // respectively.
                            actual_type = actual_type
                                .replace("{integer}", "i32")
                                .replace("{float}", "f32");
                            self.variable_states
                                .get_mut(variable_name)
                                .unwrap()
                                .type_name = actual_type;
                            retry = true;
                        } else {
                            bail!("Got error {} but failed to parse actual type", error_code);
                        }
                    } else if error_code == "E0382" {
                        // Use of moved value.
                        let old_move_state = std::mem::replace(
                            &mut self
                                .variable_states
                                .get_mut(variable_name)
                                .unwrap()
                                .move_state,
                            VariableMoveState::MovedIntoCatchUnwind,
                        );
                        if old_move_state == VariableMoveState::MovedIntoCatchUnwind {
                            // Variable is truly moved, forget about it.
                            self.variable_states.remove(variable_name);
                        }
                        retry = true;
                    } else if error_code == "E0425" {
                        // cannot find value in scope.
                        self.variable_states.remove(variable_name);
                        retry = true;
                    } else if error_code == "E0603" {
                        if let Some(variable_state) = self.variable_states.remove(variable_name) {
                            bail!(
                            "Failed to determine type of variable `{}`. rustc suggested type \
                             {}, but that's private. Sometimes adding an extern crate will help \
                             rustc suggest the correct public type name, or you can give an \
                             explicit type.",
                            variable_name,
                            variable_state.type_name
                        );
                        }
                    }
                }
                CodeOrigin::AssertCopyType { variable_name } => {
                    if error_code == "E0277" {
                        if let Some(state) = self.variable_states.get_mut(variable_name) {
                            state.is_copy_type = false;
                            retry = true;
                        }
                    }
                }
                CodeOrigin::WithFallback(fallback) => {
                    user_code.apply_fallback(fallback);
                    retry = true;
                }
                _ => {}
            }
        }
        Ok(retry)
    }

    fn record_new_locals(&mut self, pat: &syn::Pat, ty: Option<&syn::Type>) {
        use syn::export::ToTokens;
        // Default new variables to some type, say String. Assuming it isn't a
        // String, we'll get a compilation error when we try to move the
        // variable into our variable store, then we'll see what type the error
        // message says and fix it up. Hacky huh? If the user gave an explicit
        // type, we'll use that for all variables in that assignment (probably
        // only correct if it's a single variable). This gives the user a way to
        // force the type if rustc is giving us a bad suggestion.
        let type_name = ty
            .map(|ty| format!("{}", ty.into_token_stream()))
            .unwrap_or_else(|| "String".to_owned());
        idents::idents_do(pat, &mut |pat_ident: &syn::PatIdent| {
            self.variable_states.insert(
                pat_ident.ident.to_string(),
                VariableState {
                    type_name: type_name.clone(),
                    is_mut: pat_ident.mutability.is_some(),
                    // All new locals will initially be defined only inside our catch_unwind
                    // block.
                    move_state: VariableMoveState::MovedIntoCatchUnwind,
                    // Assume it's copy until we find out it's not.
                    is_copy_type: true,
                },
            );
        });
    }

    fn store_variable_statements(&mut self, move_state: &VariableMoveState) -> CodeBlock {
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

    // Returns code to load values from the variable store back into their variables.
    fn load_variable_statements(&mut self) -> CodeBlock {
        let mut statements = CodeBlock::new();
        for (var_name, var_state) in &self.variable_states {
            let mutability = if var_state.is_mut { "mut " } else { "" };
            statements.load_variable(format!(
                "let {}{} = evcxr_variable_store.take_variable::<{}>(stringify!({}));",
                mutability, var_name, var_state.type_name, var_name
            ));
        }
        statements
    }

    fn create_new_module(&mut self, previous_module: Option<&Module>) -> Result<Module, Error> {
        let crate_name = format!("user_code_{}{}", self.crate_suffix, self.build_num);
        self.build_num += 1;
        Module::new(self, &crate_name, previous_module)
    }

    // In a background thread, compile our next crate using the code from the
    // previous crate. At the time of writing (rustc 1.28.0), this appears to
    // cut our next compilation from about 340ms to 230ms. When experimenting
    // outside of evcxr, it appears that cargo build is slower if you've just
    // renamed your crate than if you haven't. I'm guessing something in the
    // incremental compilation cache includes the crate name. So after we've
    // finished a compilation, we get a head start on compiling a crate with our
    // next crate name.
    fn warm_up_next_module(
        &mut self,
        code_block: &CodeBlock,
        previous_module: &Module,
    ) -> Result<(), Error> {
        use std::sync::mpsc::channel;
        if self.next_module.lock().unwrap().is_some() {
            return Ok(());
        }
        let (started_sender, started_receiver) = channel();
        let mut module = self.create_new_module(Some(previous_module))?;
        module.write_cargo_toml(self)?;
        std::thread::spawn({
            let next_module_arc = Arc::clone(&self.next_module);
            let code_block = code_block.clone();
            move || {
                let mut next_module_arc_lock = next_module_arc.lock().unwrap();
                started_sender.send(()).unwrap();
                let _ = module.compile(&code_block);
                *next_module_arc_lock = Some(module);
                // Argh. If we don't sleep for a bit, then tests fail. It seems
                // that if we ask cargo to compile, then straight away update
                // the source, then compile again straight away, cargo thinks
                // that the source hasn't changed and doesn't actually rebuild
                // with the new source. Looks like the time precision for mtimes
                // on my file system is O(several ms) and cargo only compares
                // mtimes to deterimine if it should rebuild.
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
        // Wait until our thread has locked the next module mutex before we
        // return. That way if we go on to compile more code, it'll be
        // guaranteed to wait for then use this module.
        started_receiver.recv().unwrap();
        Ok(())
    }

    fn take_next_module(&mut self) -> Option<Module> {
        self.next_module.lock().unwrap().take()
    }

    // Returns an iterator over the loaded modules.
    pub(crate) fn modules_iter(&self) -> impl Iterator<Item = &Module> + Clone {
        self.state.modules.iter().map(|m| &*m.module)
    }

    fn get_imports(&self) -> CodeBlock {
        let mut extern_stmts = CodeBlock::new();
        let mut use_stmts = CodeBlock::new();
        for module_state in &self.state.modules {
            let crate_name = &module_state.module.crate_name;
            let defined_names = &module_state.defined_names;
            // We still import the crate, even if all its defined names have
            // been superceeded by later crates since there might be variables
            // with types defined in this crate.
            extern_stmts = extern_stmts.generated(format!("extern crate {};\n", crate_name));
            if !defined_names.is_empty() {
                use_stmts = use_stmts.generated(format!(
                    "use {}::{{{}}};\n",
                    crate_name,
                    defined_names.join(",")
                ));
            }
        }
        for stmt in &self.state.extern_crate_stmts {
            extern_stmts = extern_stmts.user_code(stmt.clone());
        }
        for user_use_stmt in &self.state.use_stmts {
            use_stmts = use_stmts.user_code(user_use_stmt.clone());
        }
        extern_stmts.add_all(use_stmts)
    }
}

#[derive(Default, Debug)]
pub struct EvalOutputs {
    pub content_by_mime_type: HashMap<String, String>,
    pub timing: Option<Duration>,
}

impl EvalOutputs {
    pub fn new() -> EvalOutputs {
        EvalOutputs {
            content_by_mime_type: HashMap::new(),
            timing: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.content_by_mime_type.is_empty()
    }

    pub fn get(&self, mime_type: &str) -> Option<&str> {
        self.content_by_mime_type.get(mime_type).map(String::as_str)
    }
}

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

#[derive(PartialEq, Eq, Debug)]
enum VariableMoveState {
    Available,
    CopiedIntoCatchUnwind,
    MovedIntoCatchUnwind,
}

struct ExecutionArtifacts {
    module: Module,
    output: EvalOutputs,
}

#[derive(Eq, PartialEq, Copy, Clone)]
enum CompilationMode {
    /// User code should be wrapped in catch_unwind and executed.
    RunAndCatchPanics,
    /// Recompile without catch_unwind to try to get better error messages. If compilation succeeds
    /// (hopefully can't happen), don't run the code - caller should return the original message.
    NoCatchExpectError,
}

/// State that is cloned then modified every time we try to compile some code. If compilation
/// succeeds, we keep the modified state, if it fails, we revert to the old state.
#[derive(Clone, Default)]
struct ContextState {
    modules: Vec<ModuleState>,
    pub(crate) external_deps: HashMap<String, ExternalCrate>,
    use_stmts: HashSet<String>,
    extern_crate_stmts: HashSet<String>,
    last_compile_dir: Option<PathBuf>,
}

#[derive(Clone)]
struct ModuleState {
    module: Arc<Module>,
    defined_names: Vec<String>,
}

impl ModuleState {
    fn new(module: Module, defined_names: Vec<String>) -> ModuleState {
        ModuleState {
            module: Arc::new(module),
            defined_names,
        }
    }
}
