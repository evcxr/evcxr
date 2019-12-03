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

use crate::code_block::{CodeBlock, CodeOrigin};
use json::{self, JsonValue};
use regex::Regex;
use std;
use std::fmt;
use std::io;

#[derive(Debug, Clone)]
pub struct CompilationError {
    message: String,
    pub json: JsonValue,
    pub(crate) code_origins: Vec<CodeOrigin>,
    spanned_messages: Vec<SpannedMessage>,
}

fn spans_in_local_source(span: &JsonValue) -> Option<&JsonValue> {
    if let Some(file_name) = span["file_name"].as_str() {
        if file_name.ends_with("lib.rs") {
            return Some(span);
        }
    }
    let expansion = &span["expansion"];
    if expansion.is_object() {
        return spans_in_local_source(&expansion["span"]);
    }
    None
}

fn get_code_origins_for_span(span: &JsonValue, code_block: &CodeBlock) -> Vec<CodeOrigin> {
    let mut code_origins = Vec::new();
    if let Some(span) = spans_in_local_source(span) {
        if let (Some(line_start), Some(line_end)) =
            (span["line_start"].as_usize(), span["line_end"].as_usize())
        {
            for line in line_start..=line_end {
                code_origins.push(code_block.origin_for_line(line));
            }
        }
    }
    code_origins
}

fn get_code_origins(json: &JsonValue, code_block: &CodeBlock) -> Vec<CodeOrigin> {
    let mut code_origins = Vec::new();
    if let JsonValue::Array(spans) = &json["spans"] {
        for span in spans {
            code_origins.extend(get_code_origins_for_span(span, code_block));
        }
    }
    code_origins
}

impl CompilationError {
    pub(crate) fn opt_new(mut json: JsonValue, code_block: &CodeBlock) -> Option<CompilationError> {
        // From Cargo 1.36 onwards, errors emitted as JSON get wrapped by Cargo.
        // Retrive the inner message emitted by the compiler.
        if json["message"].is_object() {
            json = json["message"].clone();
        }
        let mut code_origins = get_code_origins(&json, code_block);
        let mut user_error_json = None;
        if let JsonValue::Array(children) = &json["children"] {
            for child in children {
                let child_origins = get_code_origins(child, code_block);
                if !code_origins.contains(&CodeOrigin::UserSupplied)
                    && child_origins.contains(&CodeOrigin::UserSupplied)
                {
                    // Use the child instead of the top-level error.
                    user_error_json = Some(child.clone());
                    code_origins = child_origins;
                    break;
                } else {
                    code_origins.extend(child_origins);
                }
            }
        }
        if let Some(user_error_json) = user_error_json {
            json = user_error_json;
        }

        let message = if let Some(message) = json["message"].as_str() {
            if message.starts_with("aborting due to")
                || message.starts_with("For more information about")
                || message.starts_with("Some errors occurred")
            {
                return None;
            }
            message.to_owned()
        } else {
            return None;
        };

        Some(CompilationError {
            spanned_messages: build_spanned_messages(&json, code_block),
            message,
            json,
            code_origins,
        })
    }

    /// Returns whether this error originated in code supplied by the user.
    pub fn is_from_user_code(&self) -> bool {
        self.code_origins.contains(&CodeOrigin::UserSupplied)
    }

    /// Returns whether this error originated in code that we generated.
    pub fn is_from_generated_code(&self) -> bool {
        self.code_origins.contains(&CodeOrigin::OtherGeneratedCode)
    }

    pub fn message(&self) -> String {
        self.message.clone()
    }

    pub fn code(&self) -> Option<&str> {
        if let JsonValue::Object(code) = &self.json["code"] {
            return code["code"].as_str();
        }
        None
    }

    pub fn explanation(&self) -> Option<&str> {
        if let JsonValue::Object(code) = &self.json["code"] {
            return code["explanation"].as_str();
        }
        None
    }

    pub fn evcxr_extra_hint(&self) -> Option<&'static str> {
        if let Some(code) = self.code() {
            Some(match code {
                "E0597" => {
                    "Values assigned to variables in Evcxr cannot contain references \
                     (unless they're static)"
                }
                _ => return None,
            })
        } else {
            None
        }
    }

    pub fn spanned_messages(&self) -> &[SpannedMessage] {
        &self.spanned_messages[..]
    }

    pub fn help(&self) -> Vec<String> {
        if let JsonValue::Array(children) = &self.json["children"] {
            children
                .iter()
                .filter_map(|child| {
                    if child["level"].as_str() != Some("help") {
                        return None;
                    }
                    child["message"].as_str().map(|s| s.to_owned())
                })
                .collect()
        } else {
            vec![]
        }
    }

    pub fn rendered(&self) -> String {
        self.json["rendered"]
            .as_str()
            .unwrap_or_else(|| "")
            .to_owned()
    }

    /// Returns the actual type indicated by the error message or None if this isn't a type error.
    pub(crate) fn get_actual_type(&self) -> Option<String> {
        // Observed formats:
        // Up to 1.40:
        //   message.children[].message
        //     "expected type `std::string::String`\n   found type `{integer}`"
        // 1.41+:
        //   message.children[].message
        //     "expected struct `std::string::String`\n     found enum `std::option::Option<std::string::String>`"
        //     "expected struct `std::string::String`\n    found tuple `({integer}, {float})`"
        //     "  expected struct `std::string::String`\nfound opaque type `impl Bar`"
        //   message.spans[].label
        //     "expected struct `std::string::String`, found integer"
        //     "expected struct `std::string::String`, found `i32`"
        lazy_static! {
            static ref TYPE_ERROR_RE: Regex =
                Regex::new(" *expected (?s:.)*found.* `(.*)`").unwrap();
        }
        if let JsonValue::Array(children) = &self.json["children"] {
            for child in children {
                if let Some(message) = child["message"].as_str() {
                    if let Some(captures) = TYPE_ERROR_RE.captures(message) {
                        return Some(captures[1].to_owned());
                    }
                }
            }
        }
        lazy_static! {
            static ref TYPE_ERROR_RE2: Regex =
                Regex::new("expected .* found (integer|float)").unwrap();
        }
        if let JsonValue::Array(spans) = &self.json["spans"] {
            for span in spans {
                if let Some(label) = span["label"].as_str() {
                    if let Some(captures) = TYPE_ERROR_RE.captures(label) {
                        return Some(captures[1].to_owned());
                    } else if let Some(captures) = TYPE_ERROR_RE2.captures(label) {
                        return Some(captures[1].to_owned());
                    }
                }
            }
        }
        None
    }
}

fn build_spanned_messages(json: &JsonValue, code_block: &CodeBlock) -> Vec<SpannedMessage> {
    let mut output_spans = Vec::new();
    if let JsonValue::Array(spans) = &json["spans"] {
        let all_lines = code_block.get_lines();
        for span_json in spans {
            output_spans.push(SpannedMessage::from_json(span_json, &all_lines, code_block));
        }
    }
    if output_spans.iter().any(|s| s.span.is_some()) {
        // If we have at least one span in the user's code, remove all spans in generated
        // code. They'll be messages like "borrowed value only lives until here", which doesn't make
        // sense to show to the user, since "here" is is code that they didn't write and can't see.
        output_spans.retain(|s| s.span.is_some());
    }
    output_spans
}

#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub start_column: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone)]
pub struct SpannedMessage {
    pub span: Option<Span>,
    pub lines: Vec<String>,
    pub label: String,
}

impl SpannedMessage {
    fn from_json(
        span_json: &JsonValue,
        all_lines: &[String],
        code_block: &CodeBlock,
    ) -> SpannedMessage {
        let mut lines = Vec::new();
        let span = if let (
            Some(file_name),
            Some(start_column),
            Some(end_column),
            Some(start_line),
            Some(end_line),
        ) = (
            span_json["file_name"].as_str(),
            span_json["column_start"].as_usize(),
            span_json["column_end"].as_usize(),
            span_json["line_start"].as_usize(),
            span_json["line_end"].as_usize(),
        ) {
            if file_name.ends_with("lib.rs") {
                if start_line >= 1 && end_line <= all_lines.len() {
                    lines.extend(all_lines[start_line - 1..end_line].iter().cloned());
                }
                if get_code_origins_for_span(span_json, code_block)
                    .iter()
                    .all(|o| *o == CodeOrigin::UserSupplied)
                {
                    Some(Span {
                        start_column,
                        end_column,
                    })
                } else {
                    // Spans within generated code won't mean anything to the user, suppress
                    // them.
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if span.is_none() {
            let expansion_span_json = &span_json["expansion"]["span"];
            if !expansion_span_json.is_empty() {
                let mut message =
                    SpannedMessage::from_json(expansion_span_json, all_lines, code_block);
                if message.span.is_some() {
                    if let Some(label) = span_json["label"].as_str() {
                        message.label = label.to_owned();
                    }
                    return message;
                }
            }
        }
        SpannedMessage {
            span,
            lines,
            label: span_json["label"]
                .as_str()
                .map(|s| s.to_owned())
                .unwrap_or_else(String::new),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    CompilationErrors(Vec<CompilationError>),
    TypeRedefinedVariablesLost(Vec<String>),
    Message(String),
    ChildProcessTerminated(String),
}

impl Error {
    pub(crate) fn without_non_reportable_errors(mut self) -> Self {
        if let Error::CompilationErrors(errors) = &mut self {
            // If we have any errors in user code then remove all errors that aren't from user
            // code.
            if errors.iter().any(|error| error.is_from_user_code()) {
                errors.retain(|error| error.is_from_user_code())
            }
        }
        self
    }
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::CompilationErrors(errors) => {
                for error in errors {
                    write!(f, "{}", error.message())?;
                }
            }
            Error::TypeRedefinedVariablesLost(variables) => {
                write!(
                    f,
                    "A type redefinition resulted in the following variables being lost: {}",
                    variables.join(", ")
                )?;
            }
            Error::Message(message) | Error::ChildProcessTerminated(message) => {
                write!(f, "{}", message)?
            }
        }
        Ok(())
    }
}

impl From<std::fmt::Error> for Error {
    fn from(error: std::fmt::Error) -> Self {
        Error::Message(error.to_string())
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Message(error.to_string())
    }
}

impl From<json::Error> for Error {
    fn from(error: json::Error) -> Self {
        Error::Message(error.to_string())
    }
}

impl<'a> From<&'a io::Error> for Error {
    fn from(error: &'a io::Error) -> Self {
        Error::Message(error.to_string())
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(error: std::str::Utf8Error) -> Self {
        Error::Message(error.to_string())
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Error::Message(message)
    }
}

impl<'a> From<&'a str> for Error {
    fn from(message: &str) -> Self {
        Error::Message(message.to_owned())
    }
}

macro_rules! bail {
    ($e:expr) => {return Err($crate::Error::from($e))};
    ($fmt:expr, $($arg:tt)+) => {return Err($crate::Error::from(format!($fmt, $($arg)+)))}
}
