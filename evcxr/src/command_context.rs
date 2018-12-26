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

use errors::{CompilationError, Error};
use {EvalContext, EvalContextOutputs, EvalOutputs};

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
        let command_context = CommandContext {
            print_timings: false,
            eval_context,
            last_errors: Vec::new(),
        };
        Ok((command_context, eval_context_outputs))
    }

    pub fn execute(&mut self, to_run: &str) -> Result<EvalOutputs, Error> {
        use std::time::Instant;
        let start = Instant::now();
        let result = if to_run.is_empty() {
            Ok(EvalOutputs::new())
        } else if to_run.starts_with(':') {
            self.process_command(to_run)
        } else {
            self.eval_context.eval(to_run)
        };
        let duration = start.elapsed();
        match result {
            Ok(mut m) => {
                if self.print_timings {
                    m.timing = Some(duration);
                }
                Ok(m)
            }
            Err(Error::CompilationErrors(errors)) => {
                self.last_errors = errors.clone();
                Err(Error::CompilationErrors(errors))
            }
            x => x,
        }
    }

    fn process_command(&mut self, line: &str) -> Result<EvalOutputs, Error> {
        use regex::Regex;
        lazy_static! {
            static ref ADD_DEP_RE: Regex = Regex::new(":dep ([^= ]+) *= *(.+)").unwrap();
        }
        if line == ":internal_debug" {
            let debug_mode = !self.eval_context.debug_mode();
            self.eval_context.set_debug_mode(debug_mode);
            text_output(format!("Internals debugging: {}", debug_mode))
        } else if line == ":vars" {
            let mut outputs = EvalOutputs::new();
            outputs
                .content_by_mime_type
                .insert("text/plain".to_owned(), self.vars_as_text());
            outputs
                .content_by_mime_type
                .insert("text/html".to_owned(), self.vars_as_html());
            Ok(outputs)
        } else if line == ":clear" {
            self.eval_context.clear().map(|_| EvalOutputs::new())
        } else if let Some(captures) = ADD_DEP_RE.captures(line) {
            self
                .eval_context
                .add_extern_crate(captures[1].to_owned(), captures[2].to_owned())
        } else if line == ":last_compile_dir" {
            if let Some(dir) = &self.eval_context.last_compile_dir() {
                text_output(format!("{:?}", dir))
            } else {
                bail!("Nothing has been compiled yet");
            }
        } else if line == ":opt" {
            let new_level = if self.eval_context.opt_level() == "2" {
                "0"
            } else {
                "2"
            };
            self.eval_context.set_opt_level(new_level)?;
            text_output(format!("Optimization: {}", self.eval_context.opt_level()))
        } else if line == ":timing" {
            self.print_timings = !self.print_timings;
            text_output(format!("Timing: {}", self.print_timings))
        } else if line == ":explain" {
            if self.last_errors.is_empty() {
                bail!("No last error to explain");
            } else {
                let mut all_explanations = String::new();
                for error in &self.last_errors {
                    if let Some(explanation) = error.explanation() {
                        all_explanations.push_str(explanation);
                    } else {
                        bail!("Sorry, last error has no explanation");
                    }
                }
                text_output(all_explanations)
            }
        } else if line == ":last_error_json" {
            let mut errors_out = String::new();
            for error in &self.last_errors {
                use std::fmt::Write;
                write!(&mut errors_out, "{}", error.json)?;
                errors_out.push('\n');
            }
            bail!(errors_out);
        } else if line == ":help" {
            text_output(
                ":vars             List bound variables and their types\n\
                 :opt              Toggle optimization\n\
                 :explain          Print explanation of last error\n\
                 :clear            Clear all state, keeping compilation cache\n\
                 :dep              Add dependency. e.g. :dep regex = \"1.0\"\n\n\
                 Mostly for development / debugging purposes:\n\
                 :last_compile_dir Print the directory in which we last compiled\n\
                 :timing           Toggle printing of how long evaluations take\n\
                 :last_error_json  Print the last compilation error as JSON (for debugging)\n\
                 :internal_debug   Toggle various internal debugging code",
            )
        } else {
            bail!("Unrecognised command {}", line);
        }
    }

    fn vars_as_text(&self) -> String {
        let mut out = String::new();
        for (var, ty) in self.eval_context.variables_and_types() {
            out.push_str(var);
            out.push_str(": ");
            out.push_str(ty);
            out.push_str("\n");
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
}

fn html_escape(input: &str, out: &mut String) {
    for ch in input.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            x => out.push(x),
        }
    }
}

fn text_output<T: Into<String>>(text: T) -> Result<EvalOutputs, Error> {
    let mut outputs = EvalOutputs::new();
    outputs
        .content_by_mime_type
        .insert("text/plain".to_owned(), text.into());
    Ok(outputs)
}
