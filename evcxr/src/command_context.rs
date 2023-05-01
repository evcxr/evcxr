// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use crate::code_block::CodeBlock;
use crate::code_block::CodeKind;
use crate::code_block::CommandCall;
use crate::code_block::Segment;
use crate::code_block::{self};
use crate::crash_guard::CrashGuard;
use crate::errors::bail;
use crate::errors::CompilationError;
use crate::errors::Error;
use crate::errors::Span;
use crate::errors::SpannedMessage;
use crate::eval_context::ContextState;
use crate::eval_context::EvalCallbacks;
use crate::rust_analyzer::Completion;
use crate::rust_analyzer::Completions;
use crate::EvalContext;
use crate::EvalContextOutputs;
use crate::EvalOutputs;
use anyhow::Result;
use once_cell::sync::OnceCell;

/// A higher level interface to EvalContext. A bit closer to a Repl. Provides commands (start with
/// ':') that alter context state or print information.
pub struct CommandContext {
    print_timings: bool,
    eval_context: EvalContext,
    last_errors: Vec<CompilationError>,
}

impl CommandContext {
    pub fn new() -> Result<(CommandContext, EvalContextOutputs), Error> {
        let (eval_context, eval_context_outputs) = EvalContext::new()?;
        let command_context = CommandContext::with_eval_context(eval_context);
        Ok((command_context, eval_context_outputs))
    }

    pub fn with_eval_context(eval_context: EvalContext) -> CommandContext {
        CommandContext {
            print_timings: false,
            eval_context,
            last_errors: Vec::new(),
        }
    }

    #[doc(hidden)]
    pub fn new_for_testing() -> (CommandContext, EvalContextOutputs) {
        let (eval_context, outputs) = EvalContext::new_for_testing();
        (Self::with_eval_context(eval_context), outputs)
    }

    pub fn execute(&mut self, to_run: &str) -> Result<EvalOutputs, Error> {
        self.execute_with_callbacks(to_run, &mut EvalCallbacks::default())
    }

    pub fn check(&mut self, code: &str) -> Result<Vec<CompilationError>, Error> {
        let (user_code, code_info) = CodeBlock::from_original_user_code(code);
        let (non_command_code, state, errors) = self.prepare_for_analysis(user_code)?;
        if !errors.is_empty() {
            // If we've got errors while preparing, probably due to bad :dep commands, then there's
            // no point running cargo check as it'd just give us additional follow-on errors which
            // would be confusing.
            return Ok(errors);
        }
        self.eval_context.check(non_command_code, state, &code_info)
    }

    pub fn process_handle(&self) -> Arc<Mutex<std::process::Child>> {
        self.eval_context.process_handle()
    }

    pub fn variables_and_types(&self) -> impl Iterator<Item = (&str, &str)> {
        self.eval_context.variables_and_types()
    }

    pub fn reset_config(&mut self) {
        self.eval_context.reset_config();
    }

    pub fn defined_item_names(&self) -> impl Iterator<Item = &str> {
        self.eval_context.defined_item_names()
    }

    pub fn execute_with_callbacks(
        &mut self,
        to_run: &str,
        callbacks: &mut EvalCallbacks,
    ) -> Result<EvalOutputs, Error> {
        let mut state = self.eval_context.state();
        state.clear_non_debug_relevant_fields();
        let mut guard = CrashGuard::new(|| {
            eprintln!(
                r#"
=============================================================================
Panic detected. Here's some useful information if you're filing a bug report.
<CODE>
{to_run}
</CODE>
<STATE>
{state:?}
</STATE>"#
            );
        });
        let result = self.execute_with_callbacks_internal(to_run, callbacks);
        guard.disarm();
        result
    }

    fn execute_with_callbacks_internal(
        &mut self,
        to_run: &str,
        callbacks: &mut EvalCallbacks,
    ) -> Result<EvalOutputs, Error> {
        use std::time::Instant;
        let mut eval_outputs = EvalOutputs::new();
        let start = Instant::now();
        let mut state = self.eval_context.state();
        let mut non_command_code = CodeBlock::new();
        let (user_code, code_info) = CodeBlock::from_original_user_code(to_run);
        for segment in user_code.segments {
            if let CodeKind::Command(command) = &segment.kind {
                eval_outputs.merge(self.execute_command(
                    command,
                    &segment,
                    &mut state,
                    &command.args,
                )?);
            } else {
                non_command_code = non_command_code.with_segment(segment);
            }
        }
        let result =
            self.eval_context
                .eval_with_callbacks(non_command_code, state, &code_info, callbacks);
        let duration = start.elapsed();
        match result {
            Ok(m) => {
                eval_outputs.merge(m);
                if self.print_timings {
                    eval_outputs.timing = Some(duration);
                }
                Ok(eval_outputs)
            }
            Err(Error::CompilationErrors(errors)) => {
                self.last_errors = errors.clone();
                Err(Error::CompilationErrors(errors))
            }
            x => x,
        }
    }

    pub fn set_opt_level(&mut self, level: &str) -> Result<(), Error> {
        self.eval_context.set_opt_level(level)
    }

    pub fn last_source(&self) -> std::io::Result<String> {
        self.eval_context.last_source()
    }

    /// Returns completions within `src` at `position`, which should be a byte offset. Note, this
    /// function requires &mut self because it mutates internal state in order to determine
    /// completions. It also assumes exclusive access to those resources. However there should be
    /// any visible side effects.
    pub fn completions(&mut self, src: &str, position: usize) -> Result<Completions> {
        let (user_code, code_info) = CodeBlock::from_original_user_code(src);
        if let Some((segment, offset)) = user_code.command_containing_user_offset(position) {
            return self.command_completions(segment, offset, position);
        }
        let (non_command_code, state, _errors) = self.prepare_for_analysis(user_code)?;
        self.eval_context
            .completions(non_command_code, state, &code_info.nodes, position)
    }

    fn prepare_for_analysis(
        &mut self,
        user_code: CodeBlock,
    ) -> Result<(CodeBlock, ContextState, Vec<CompilationError>)> {
        let mut non_command_code = CodeBlock::new();
        let mut state = self.eval_context.state();
        let mut errors = Vec::new();
        for segment in user_code.segments {
            if let CodeKind::Command(command) = &segment.kind {
                if let Err(error) =
                    self.process_command(command, &segment, &mut state, &command.args, true)
                {
                    errors.push(error);
                }
            } else {
                non_command_code = non_command_code.with_segment(segment);
            }
        }
        self.eval_context.write_cargo_toml(&state)?;
        Ok((non_command_code, state, errors))
    }

    fn command_completions(
        &self,
        segment: &Segment,
        offset: usize,
        full_position: usize,
    ) -> Result<Completions> {
        let existing = &segment.code[0..offset];
        let mut completions = Completions {
            start_offset: full_position - offset,
            end_offset: full_position,
            ..Completions::default()
        };
        for cmd in Self::commands_by_name().keys() {
            if cmd.starts_with(existing) {
                completions.completions.push(Completion {
                    code: (*cmd).to_owned(),
                })
            }
        }
        Ok(completions)
    }

    fn load_config(&mut self, quiet: bool) -> Result<EvalOutputs, Error> {
        let mut outputs = EvalOutputs::new();
        if let Some(config_dir) = crate::config_dir() {
            let config_file = config_dir.join("init.evcxr");
            if config_file.exists() {
                if !quiet {
                    println!("Loading startup commands from {config_file:?}");
                }
                let contents = std::fs::read_to_string(config_file)?;
                for line in contents.lines() {
                    outputs.merge(self.execute(line)?);
                }
            }
            // Note: Loaded *after* init.evcxr so that it can access `:dep`s (or
            // any other state changed by :commands) specified in the init file.
            let prelude_file = config_dir.join("prelude.rs");
            if prelude_file.exists() {
                if !quiet {
                    println!("Executing prelude from {prelude_file:?}");
                }
                let prelude = std::fs::read_to_string(prelude_file)?;
                outputs.merge(self.execute(&prelude)?);
            }
        }
        Ok(outputs)
    }

    fn execute_command(
        &mut self,
        command: &CommandCall,
        segment: &Segment,
        state: &mut ContextState,
        args: &Option<String>,
    ) -> Result<EvalOutputs, Error> {
        self.process_command(command, segment, state, args, false)
            .map_err(|err| Error::CompilationErrors(vec![err]))
    }

    fn process_command(
        &mut self,
        command_call: &CommandCall,
        segment: &Segment,
        state: &mut ContextState,
        args: &Option<String>,
        analysis_mode: bool,
    ) -> Result<EvalOutputs, CompilationError> {
        if let Some(command) = Self::commands_by_name().get(command_call.command.as_str()) {
            let result = match &command.analysis_callback {
                Some(analysis_callback) if analysis_mode => (analysis_callback)(self, state, args),
                _ => (command.callback)(self, state, args),
            };
            result.map_err(|error| {
                // Span from the start of the arguments to the end of the arguments, or if no
                // arguments are found, span the command. We look for the first non-space character
                // after a space is found.
                let mut found_space = false;
                let start_byte = segment
                    .code
                    .bytes()
                    .enumerate()
                    .find(|(_index, byte)| {
                        if *byte == b' ' {
                            found_space = true;
                            return false;
                        }
                        found_space
                    })
                    .map(|(index, _char)| index)
                    .unwrap_or(0);
                let start_column = code_block::count_columns(&segment.code[..start_byte]) + 1;
                let end_column = code_block::count_columns(&segment.code);
                CompilationError::from_segment_span(
                    segment,
                    SpannedMessage::from_segment_span(
                        segment,
                        Span::from_command(command_call, start_column, end_column),
                    ),
                    error.to_string(),
                )
            })
        } else {
            Err(CompilationError::from_segment_span(
                segment,
                SpannedMessage::from_segment_span(
                    segment,
                    Span::from_command(
                        command_call,
                        1,
                        code_block::count_columns(&command_call.command) + 1,
                    ),
                ),
                format!("Unrecognised command {}", command_call.command),
            ))
        }
    }

    fn commands_by_name() -> &'static HashMap<&'static str, AvailableCommand> {
        static COMMANDS_BY_NAME: OnceCell<HashMap<&'static str, AvailableCommand>> =
            OnceCell::new();
        COMMANDS_BY_NAME.get_or_init(|| {
            CommandContext::create_commands()
                .into_iter()
                .map(|command| (command.name, command))
                .collect()
        })
    }

    fn create_commands() -> Vec<AvailableCommand> {
        vec![
            AvailableCommand::new(
                ":internal_debug",
                "Toggle various internal debugging code",
                |_ctx, state, _args| {
                    let debug_mode = !state.debug_mode();
                    state.set_debug_mode(debug_mode);
                    text_output(format!("Internals debugging: {debug_mode}"))
                },
            ),
            AvailableCommand::new(
                ":load_config",
                "Reloads startup configuration files. Accepts optional flag `--quiet` to suppress logging.",
                |ctx, state, args| {
                    let quiet = args.as_ref().map(String::as_str) == Some("--quiet");
                    let result = ctx.load_config(quiet);
                    *state = ctx.eval_context.state();
                    result
                },
            )
            .disable_in_analysis(),
            AvailableCommand::new(":version", "Print Evcxr version", |_ctx, _state, _args| {
                text_output(env!("CARGO_PKG_VERSION"))
            }),
            AvailableCommand::new(
                ":vars",
                "List bound variables and their types",
                |ctx, _state, _args| {
                    Ok(EvalOutputs::text_html(
                        ctx.vars_as_text(),
                        ctx.vars_as_html(),
                    ))
                },
            ),
            AvailableCommand::new(
                ":type",
                "Show variable type",
                |ctx, _state, args| {
                    ctx.var_type(args)
                },
            ),
            AvailableCommand::new(
                ":t",
                "Short version of :type",
                |ctx, _state, args| {
                    ctx.var_type(args)
                },
            ),
            AvailableCommand::new(
                ":preserve_vars_on_panic",
                "Try to keep vars on panic (0/1)",
                |_ctx, state, args| {
                    state
                        .set_preserve_vars_on_panic(args.as_ref().map(String::as_str) == Some("1"));
                    text_output(format!(
                        "Preserve vars on panic: {}",
                        state.preserve_vars_on_panic()
                    ))
                },
            ),
            AvailableCommand::new(
                ":clear",
                "Clear all state, keeping compilation cache",
                |ctx, state, _args| {
                    ctx.eval_context.clear().map(|_| {
                        *state = ctx.eval_context.state();
                        EvalOutputs::new()
                    })
                },
            )
            .with_analysis_callback(|ctx, state, _args| {
                *state = ctx.eval_context.cleared_state();
                Ok(EvalOutputs::default())
            }),
            AvailableCommand::new(
                ":dep",
                "Add dependency. e.g. :dep regex = \"1.0\"",
                |_ctx, state, args| process_dep_command(state, args),
            ),
            AvailableCommand::new(
                ":last_compile_dir",
                "Print the directory in which we last compiled",
                |ctx, _state, _args| {
                    text_output(format!("{:?}", ctx.eval_context.last_compile_dir()))
                },
            ),
            AvailableCommand::new(
                ":opt",
                "Set optimization level (0/1/2)",
                |_ctx, state, args| {
                    let new_level = if let Some(n) = args {
                        n
                    } else if state.opt_level() == "2" {
                        "0"
                    } else {
                        "2"
                    };
                    state.set_opt_level(new_level)?;
                    text_output(format!("Optimization: {}", state.opt_level()))
                },
            ),
            AvailableCommand::new(
                ":fmt",
                "Set output formatter (default: {:?})",
                |_ctx, state, args| {
                    let new_format = if let Some(f) = args { f } else { "{:?}" };
                    state.set_output_format(new_format.to_owned());
                    text_output(format!("Output format: {}", state.output_format()))
                },
            ),
            AvailableCommand::new(
                ":types",
                "Toggle printing of types",
                |_ctx, state, _args| {
                    state.set_display_types(!state.display_types());
                    text_output(format!("Types: {}", state.display_types()))
                },
            ),
            AvailableCommand::new(
                ":efmt",
                "Set the formatter for errors returned by ?",
                |_ctx, state, args| {
                    if let Some(f) = args {
                        state.set_error_format(f)?;
                    }
                    text_output(format!(
                        "Error format: {} (errors must implement {})",
                        state.error_format(),
                        state.error_format_trait()
                    ))
                },
            ),
            AvailableCommand::new(
                ":toolchain",
                "Set which toolchain to use (e.g. nightly)",
                |_ctx, state, args| {
                    if let Some(arg) = args {
                        state.set_toolchain(arg);
                    }
                    text_output(format!("Toolchain: {}", state.toolchain()))
                },
            ),
            AvailableCommand::new(
                ":offline",
                "Set offline mode when invoking cargo (0/1)",
                |_ctx, state, args| {
                    state.set_offline_mode(args.as_ref().map(String::as_str) == Some("1"));
                    text_output(format!("Offline mode: {}", state.offline_mode()))
                },
            ),
            AvailableCommand::new(
                ":quit",
                "Quit evaluation and exit",
                |_ctx, _state, _args| std::process::exit(0),
            )
            .disable_in_analysis(),
            AvailableCommand::new(
                ":timing",
                "Toggle printing of how long evaluations take",
                |ctx, _state, _args| {
                    ctx.print_timings = !ctx.print_timings;
                    text_output(format!("Timing: {}", ctx.print_timings))
                },
            ),
            AvailableCommand::new(
                ":time_passes",
                "Toggle printing of rustc pass times (requires nightly)",
                |_ctx, state, _args| {
                    state.set_time_passes(!state.time_passes());
                    text_output(format!("Time passes: {}", state.time_passes()))
                },
            ),
            AvailableCommand::new(
                ":sccache",
                "Set whether to use sccache (0/1).",
                |_ctx, state, args| {
                    state.set_sccache(args.as_ref().map(String::as_str) != Some("0"))?;
                    text_output(format!("sccache: {}", state.sccache()))
                },
            ),
            AvailableCommand::new(
                ":linker",
                "Set/print linker. Supported: system, lld, mold",
                |_ctx, state, args| {
                    if let Some(linker) = args {
                        state.set_linker(linker.to_owned());
                    }
                    text_output(format!("linker: {}", state.linker()))
                },
            ),
            AvailableCommand::new(
                ":explain",
                "Print explanation of last error",
                |ctx, _state, _args| {
                    if ctx.last_errors.is_empty() {
                        bail!("No last error to explain");
                    } else {
                        let mut all_explanations = String::new();
                        for error in &ctx.last_errors {
                            if let Some(explanation) = error.explanation() {
                                all_explanations.push_str(explanation);
                            } else {
                                bail!("Sorry, last error has no explanation");
                            }
                        }
                        text_output(all_explanations)
                    }
                },
            ),
            AvailableCommand::new(
                ":last_error_json",
                "Print the last compilation error as JSON (for debugging)",
                |ctx, _state, _args| {
                    let mut errors_out = String::new();
                    for error in &ctx.last_errors {
                        use std::fmt::Write;
                        write!(errors_out, "{}", error.json)?;
                        errors_out.push('\n');
                    }
                    bail!(errors_out);
                },
            ),
            AvailableCommand::new(":help", "Print command help", |_ctx, _state, _args| {
                use std::fmt::Write;
                let mut text = String::new();
                let mut html = String::new();
                writeln!(html, "<table>")?;
                let mut commands = CommandContext::create_commands();
                commands.sort_by(|a, b| a.name.cmp(b.name));
                for cmd in commands {
                    writeln!(text, "{:<17} {}", cmd.name, cmd.short_description).unwrap();
                    writeln!(
                        html,
                        "<tr><td>{}</td><td>{}</td></tr>",
                        cmd.name, cmd.short_description
                    )?;
                }
                writeln!(html, "</table>")?;
                Ok(EvalOutputs::text_html(text, html))
            }),
        ]
    }

    fn vars_as_text(&self) -> String {
        let mut out = String::new();
        for (var, ty) in self.eval_context.variables_and_types() {
            out.push_str(var);
            out.push_str(": ");
            out.push_str(ty);
            out.push('\n');
        }
        out
    }

    fn vars_as_html(&self) -> String {
        let mut out = String::new();
        out.push_str("<table><tr><th>Variable</th><th>Type</th></tr>");
        for (var, ty) in self.eval_context.variables_and_types() {
            out.push_str("<tr><td>");
            html_escape(var, &mut out);
            out.push_str("</td><td>");
            html_escape(ty, &mut out);
            out.push_str("</td><tr>");
        }
        out.push_str("</table>");
        out
    }

    fn var_type(&self, args: &Option<String>) -> Result<EvalOutputs, Error> {
        let args = if let Some(x) = args {
            x.trim()
        } else {
            bail!("Variable name required")
        };

        let mut out = None;
        for (var, ty) in self.eval_context.variables_and_types() {
            if var == args {
                out = Some(ty.to_owned());
                break;
            }
        }

        if let Some(out) = out {
            let out = format!("{args}: {out}");
            Ok(EvalOutputs::text_html(out.clone(), out))
        } else {
            bail!("Variable does not exist: {}", args)
        }
    }
}

fn process_dep_command(
    state: &mut ContextState,
    args: &Option<String>,
) -> Result<EvalOutputs, Error> {
    use regex::Regex;
    let Some(args) = args else {
        bail!(":dep requires arguments")
    };
    static DEP_RE: OnceCell<Regex> = OnceCell::new();
    let dep_re = DEP_RE.get_or_init(|| Regex::new("^([^= ]+) *(?:= *(.+))?$").unwrap());
    if let Some(captures) = dep_re.captures(args) {
        if captures[1].starts_with('.') || captures[1].starts_with('/') {
            state.add_local_dep(&captures[1])?;
        } else {
            state.add_dep(
                &captures[1],
                captures.get(2).map_or("\"*\"", |m| m.as_str()),
            )?;
        }
        Ok(EvalOutputs::new())
    } else {
        bail!("Invalid :dep command. Expected: name = ... or just name");
    }
}

type CallbackFn = dyn Fn(&mut CommandContext, &mut ContextState, &Option<String>) -> Result<EvalOutputs, Error>
    + 'static
    + Sync
    + Send;

struct AvailableCommand {
    name: &'static str,
    short_description: &'static str,
    callback: Box<CallbackFn>,
    /// If `Some`, this callback will be run when preparing for analysis instead of `callback`.
    analysis_callback: Option<Box<CallbackFn>>,
}

impl AvailableCommand {
    fn new(
        name: &'static str,
        short_description: &'static str,
        callback: impl Fn(
                &mut CommandContext,
                &mut ContextState,
                &Option<String>,
            ) -> Result<EvalOutputs, Error>
            + 'static
            + Sync
            + Send,
    ) -> AvailableCommand {
        AvailableCommand {
            name,
            short_description,
            callback: Box::new(callback),
            analysis_callback: None,
        }
    }

    fn with_analysis_callback(
        mut self,
        callback: impl Fn(
                &mut CommandContext,
                &mut ContextState,
                &Option<String>,
            ) -> Result<EvalOutputs, Error>
            + 'static
            + Sync
            + Send,
    ) -> Self {
        self.analysis_callback = Some(Box::new(callback));
        self
    }

    fn disable_in_analysis(self) -> Self {
        self.with_analysis_callback(|_ctx, _state, _args| Ok(EvalOutputs::default()))
    }
}

fn html_escape(input: &str, out: &mut String) {
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            x => out.push(x),
        }
    }
}

fn text_output<T: Into<String>>(text: T) -> Result<EvalOutputs, Error> {
    let mut outputs = EvalOutputs::new();
    let mut content = text.into();
    content.push('\n');
    outputs
        .content_by_mime_type
        .insert("text/plain".to_owned(), content);
    Ok(outputs)
}
