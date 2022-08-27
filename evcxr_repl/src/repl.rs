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

use super::scan::validate_source_fragment;
use super::scan::FragmentValidity;
use crate::bginit::BgInitMutex;
use colored::*;
use evcxr::CommandContext;
use evcxr::Completions;
use evcxr::Error;
use rustyline::completion::Completer;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::ValidationContext;
use rustyline::validate::ValidationResult;
use rustyline::validate::Validator;
use rustyline::Context;
use rustyline::Helper;
use std::borrow::Cow;
use std::sync::Arc;

pub struct EvcxrRustylineHelper {
    command_context: Arc<BgInitMutex<Result<CommandContext, Error>>>,
}

impl EvcxrRustylineHelper {
    pub fn new(command_context: Arc<BgInitMutex<Result<CommandContext, Error>>>) -> Self {
        Self { command_context }
    }
}

// Have to implement a bunch of traits as mostly noop...

impl Hinter for EvcxrRustylineHelper {
    type Hint = String;
}

impl Completer for EvcxrRustylineHelper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        let completions = match &mut *self.command_context.lock() {
            Ok(command_context) => command_context
                .completions(line, pos)
                .unwrap_or_else(|_| Completions::default()),
            Err(e) => {
                return Err(rustyline::error::ReadlineError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.clone(),
                )))
            }
        };
        let res: Vec<String> = completions
            .completions
            .into_iter()
            .map(|c| c.code)
            .collect();
        Ok((completions.start_offset, res))
    }
}

impl Highlighter for EvcxrRustylineHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        prompt.yellow().to_string().into()
    }
}

impl Validator for EvcxrRustylineHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> Result<ValidationResult, ReadlineError> {
        let input = ctx.input();
        // If a user is hammering on the enter key, lets pass things along to
        // rustc. This is an escape hatch for the case where *we* know (well,
        // think) the source is incomplete, but the user doesn't. It also makes
        // bugs in our code less disasterous.
        if input.ends_with("\n\n") {
            return Ok(ValidationResult::Valid(None));
        }
        match validate_source_fragment(input) {
            FragmentValidity::Incomplete => Ok(ValidationResult::Incomplete),
            FragmentValidity::Invalid => {
                // Hrm... AFAICT if we return Invalid here, we don't get to run
                // it. `rustc` is likely to be able to provide a better error
                // message than us, so...
                Ok(ValidationResult::Valid(None))
            }
            FragmentValidity::Valid => Ok(ValidationResult::Valid(None)),
        }
    }

    // We actually work with this on for the most part, but it seems incomplete
    // in rustyline, so disable it (explicitly). It's unclear how desirable
    // it is without the ability to e.g. highlight the mismatched bracket or
    // whatever.
    fn validate_while_typing(&self) -> bool {
        false
    }
}

impl Helper for EvcxrRustylineHelper {}
