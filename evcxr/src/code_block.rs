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

use crate::statement_splitter::{self, UserCodeMetadata};
use anyhow::{anyhow, Result};
use std;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Segment {
    pub(crate) kind: CodeKind,
    pub(crate) code: String,
    num_lines: usize,
}

impl Segment {
    fn new(kind: CodeKind, mut code: String) -> Segment {
        if !code.ends_with('\n') {
            code.push('\n');
        }
        Segment {
            kind,
            num_lines: num_lines(&code),
            code,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Command {
    pub(crate) command: String,
    pub(crate) args: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum CodeKind {
    /// The code was supplied by the user. Errors should be reported to the user.
    OriginalUserCode(UserCodeMetadata),
    /// User code for which we don't track offsets.
    OtherUserCode,
    /// Code is packing a variable into the variable store. Failure modes include (a) incorrect type
    /// (b) variable has been moved (c) non-static lifetime.
    PackVariable {
        variable_name: String,
    },
    /// Used to check if a variable implements Copy.
    AssertCopyType {
        variable_name: String,
    },
    /// A line of code that has a fallback to be used in case the supplied line fails to compile.
    WithFallback(CodeBlock),
    /// Code that we generated, but which we don't expect errors from. If we get errors there's not
    /// much we can do besides give the user as much information as we can, appologise and ask to
    /// file a bug report.
    OtherGeneratedCode,
    /// We had trouble determining what the error applied to.
    Command(Command),
    Unknown,
}

impl CodeKind {
    /// Returns whether self is a WithFallback where the replacement is equal to the supplied
    /// fallback. Using the whole fallback as an "ID" may seem a bit heavy handed, but I doubt if
    /// this is likely to ever be a performance consideration. Also, in theory we should perhaps use
    /// the code being replaced as the ID, but in practice the fallback is equally unique.
    fn equals_fallback(&self, fallback: &CodeBlock) -> bool {
        if let CodeKind::WithFallback(self_fallback) = self {
            return self_fallback == fallback;
        }
        false
    }

    pub(crate) fn is_user_supplied(&self) -> bool {
        matches!(self, CodeKind::OriginalUserCode(_) | CodeKind::OtherUserCode)
    }
}

fn num_lines(code: &str) -> usize {
    code.chars().filter(|ch| *ch == '\n').count()
}

/// Represents a unit of code. This may be code that the user supplied, in which case it might
/// include evcxr commands. By the time the code is ready to send to the compiler, it shouldn't have
/// any evcxr commands and should have additional supporting code for things like packing and
/// unpacking variables.
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub(crate) struct CodeBlock {
    pub(crate) segments: Vec<Segment>,
}

impl CodeBlock {
    pub(crate) fn new() -> CodeBlock {
        Self::default()
    }

    /// Passes `self` as an owned value to `f`, replacing `self` with the return
    /// value of `f` once done. This is a convenience for when we only have a
    /// &mut, not an owned value.
    pub(crate) fn modify<F: FnOnce(CodeBlock) -> CodeBlock>(&mut self, f: F) {
        let mut block = std::mem::replace(self, CodeBlock::new());
        block = f(block);
        *self = block;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub(crate) fn with_segment(mut self, segment: Segment) -> Self {
        self.segments.push(segment);
        self
    }

    pub(crate) fn with<T: Into<String>>(mut self, origin: CodeKind, code: T) -> Self {
        self.segments.push(Segment::new(origin, code.into()));
        self
    }

    pub(crate) fn code_with_fallback<T: Into<String>>(self, code: T, fallback: CodeBlock) -> Self {
        self.with(CodeKind::WithFallback(fallback), code)
    }

    pub(crate) fn generated<T: Into<String>>(self, code: T) -> Self {
        self.with(CodeKind::OtherGeneratedCode, code)
    }

    pub(crate) fn other_user_code(self, user_code: String) -> CodeBlock {
        self.with(CodeKind::OtherUserCode, user_code)
    }

    pub(crate) fn original_user_code(mut self, user_code: &str) -> CodeBlock {
        use regex::Regex;
        lazy_static! {
            static ref COMMAND_RE: Regex = Regex::new("^ *(:[^ ]+)( +(.*))?$").unwrap();
        }
        for line in user_code.lines() {
            // We only accept commands up until the first non-command.
            if let Some(captures) = COMMAND_RE.captures(line) {
                self = self.with(
                    CodeKind::Command(Command {
                        command: captures[1].to_owned(),
                        args: captures.get(3).map(|m| m.as_str().to_owned()),
                    }),
                    line,
                );
            } else if line.trim().is_empty() {
                // Ignore blank lines, otherwise we can't have blank lines before :dep commands.
            } else {
                // Anything else, we treat as Rust code to be executed. Since we don't accept commands after Rust code, we're done looking for commands.
                let non_command_start_byte = line.as_ptr() as usize - user_code.as_ptr() as usize;
                for (statement_code, mut meta) in
                    statement_splitter::split_into_statements(&user_code[non_command_start_byte..])
                {
                    meta.start_byte += non_command_start_byte;
                    self = self.with(CodeKind::OriginalUserCode(meta), statement_code);
                }
                break;
            }
        }
        self
    }

    /// Tries to convert a user-code offset into an output code offset. For this to work as
    /// expected, there should have been a single call to original_user_code and user_code_offset
    /// should refer to a byte offset within the value that was passed.
    pub(crate) fn user_offset_to_output_offset(&self, user_code_offset: usize) -> Result<usize> {
        let mut bytes_seen = 0;
        self.segments
            .iter()
            .find_map(|segment| {
                if let CodeKind::OriginalUserCode(meta) = &segment.kind {
                    if user_code_offset >= meta.start_byte
                        && user_code_offset <= meta.start_byte + segment.code.len()
                    {
                        return Some(bytes_seen + user_code_offset - meta.start_byte);
                    }
                }
                bytes_seen += segment.code.len();
                None
            })
            .ok_or_else(|| anyhow!("Offset {} doesn't refer to user code", user_code_offset))
    }

    pub(crate) fn output_offset_to_user_offset(&self, output_offset: usize) -> Result<usize> {
        let mut bytes_seen = 0;
        self.segments
            .iter()
            .find_map(|segment| {
                if let CodeKind::OriginalUserCode(meta) = &segment.kind {
                    if output_offset >= bytes_seen
                        && output_offset <= bytes_seen + segment.code.len()
                    {
                        return Some(meta.start_byte + output_offset - bytes_seen);
                    }
                }
                bytes_seen += segment.code.len();
                None
            })
            .ok_or_else(|| anyhow!("Output offset {} doesn't refer to user code", output_offset))
    }

    pub(crate) fn load_variable(&mut self, code: String) {
        self.segments
            .push(Segment::new(CodeKind::OtherGeneratedCode, code));
    }

    pub(crate) fn pack_variable(&mut self, variable_name: String, code: String) {
        self.segments
            .push(Segment::new(CodeKind::PackVariable { variable_name }, code));
    }

    pub(crate) fn assert_copy_variable(&mut self, variable_name: String, code: String) {
        self.segments.push(Segment::new(
            CodeKind::AssertCopyType { variable_name },
            code,
        ));
    }

    pub(crate) fn add_all(mut self, other: CodeBlock) -> Self {
        self.segments.extend(other.segments);
        self
    }

    pub(crate) fn code_string(&self) -> String {
        let mut output = String::new();
        for segment in &self.segments {
            output.push_str(&segment.code);
        }
        output
    }

    /// Returns the segment type for the specified line (starts from 1). Out-of-range indices will
    /// return type Unknown.
    pub(crate) fn origin_for_line(&self, line_number: usize) -> &CodeKind {
        if line_number == 0 {
            return &CodeKind::Unknown;
        }
        let mut current_line_number = 1;
        for segment in &self.segments {
            current_line_number += segment.num_lines;
            if current_line_number > line_number {
                return &segment.kind;
            }
        }
        &CodeKind::Unknown
    }

    pub(crate) fn get_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for segment in &self.segments {
            lines.extend(segment.code.lines().map(str::to_owned));
        }
        lines
    }

    pub(crate) fn apply_fallback(&mut self, fallback: &CodeBlock) {
        let mut replacement_segments = Vec::new();
        for segment in std::mem::replace(&mut self.segments, Vec::new()) {
            if segment.kind.equals_fallback(fallback) {
                replacement_segments.extend(fallback.segments.clone());
            } else {
                replacement_segments.push(segment);
            }
        }
        self.segments = replacement_segments;
    }
}

#[cfg(test)]
mod test {
    use super::{CodeBlock, CodeKind, UserCodeMetadata};

    #[test]
    fn basic_usage() {
        let user_code = "l3";
        let mut code = CodeBlock::new()
            .generated("l1\nl2")
            .original_user_code(user_code)
            .add_all(CodeBlock::new().generated("l4"));
        code.pack_variable("v".to_owned(), "l5".to_owned());
        assert_eq!(code.code_string(), "l1\nl2\nl3\nl4\nl5\n");
        assert_eq!(code.segments.len(), 4);
        assert_eq!(
            code.segments
                .iter()
                .map(|s| s.num_lines)
                .collect::<Vec<_>>(),
            vec![2, 1, 1, 1]
        );
        assert_eq!(code.origin_for_line(0), &CodeKind::Unknown);
        assert_eq!(code.origin_for_line(1), &CodeKind::OtherGeneratedCode);
        assert_eq!(code.origin_for_line(2), &CodeKind::OtherGeneratedCode);
        assert_eq!(
            code.origin_for_line(3),
            &CodeKind::OriginalUserCode(UserCodeMetadata { start_byte: 0 })
        );
        assert_eq!(code.origin_for_line(4), &CodeKind::OtherGeneratedCode);
        assert_eq!(
            code.origin_for_line(5),
            &CodeKind::PackVariable {
                variable_name: "v".to_owned()
            }
        );
        assert_eq!(code.origin_for_line(6), &CodeKind::Unknown);

        assert_eq!(
            &code.code_string()[code.user_offset_to_output_offset(0).unwrap()
                ..code.user_offset_to_output_offset(user_code.len()).unwrap()],
            user_code
        );
    }
}
