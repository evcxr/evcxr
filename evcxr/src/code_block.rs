// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::statement_splitter;
use anyhow::anyhow;
use anyhow::Result;
use once_cell::sync::OnceCell;
use ra_ap_syntax::SyntaxNode;
use regex::Regex;
use statement_splitter::OriginalUserCode;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Segment {
    pub(crate) kind: CodeKind,
    pub(crate) code: String,
    num_lines: usize,
    /// Only present for original user code. Provides ordering and identity to the segments that
    /// came from the user.
    pub(crate) sequence: Option<usize>,
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
            sequence: None,
        }
    }
}

/// Information about the code the user supplied.
pub(crate) struct UserCodeInfo<'a> {
    pub(crate) nodes: Vec<SyntaxNode>,
    pub(crate) original_lines: Vec<&'a str>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct CommandCall {
    pub(crate) command: String,
    pub(crate) args: Option<String>,
    start_byte: usize,
    pub(crate) line_number: usize,
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
    /// A line of code that has a fallback to be used in case the supplied line fails to compile.
    WithFallback(CodeBlock),
    /// Code that we generated, but which we don't expect errors from. If we get errors there's not
    /// much we can do besides give the user as much information as we can, apologise and ask to
    /// file a bug report.
    OtherGeneratedCode,
    /// We had trouble determining what the error applied to.
    Command(CommandCall),
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
        matches!(
            self,
            CodeKind::OriginalUserCode(_) | CodeKind::OtherUserCode | CodeKind::Command(_)
        )
    }
}

fn num_lines(code: &str) -> usize {
    code.chars().filter(|ch| *ch == '\n').count()
}

pub(crate) fn count_columns(code: &str) -> usize {
    // We use characters here, not graphemes because seems to be how columns are counted by the rust
    // compiler, which we need to be consistent with. It also works well with the inline error
    // reporting in Jupyter notebook. It doesn't work so well for the terminal, which needs
    // graphemes, but that is handled by the REPL.
    code.chars().count()
}

/// Information about some code that the user supplied.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct UserCodeMetadata {
    /// The starting byte in the code as the user wrote it.
    pub(crate) start_byte: usize,
    pub(crate) node_index: usize,
    /// The line number (starting from 1) in the original user code on which this code starts.
    pub(crate) start_line: usize,
    /// The number of graphemes (not characters or bytes) on the line from which
    /// this code came that are prior to and not included in this code.
    pub(crate) column_offset: usize,
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
        let mut block = std::mem::take(self);
        block = f(block);
        *self = block;
    }

    pub(crate) fn commit_old_user_code(&mut self) {
        for segment in self.segments.iter_mut() {
            if matches!(segment.kind, CodeKind::OriginalUserCode(_)) {
                segment.kind = CodeKind::OtherUserCode;
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub(crate) fn segment_with_index(&self, index: usize) -> Option<&Segment> {
        self.segments
            .iter()
            .find(|segment| segment.sequence == Some(index))
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

    pub(crate) fn from_original_user_code(user_code: &str) -> (CodeBlock, UserCodeInfo) {
        static COMMAND_RE: OnceCell<Regex> = OnceCell::new();
        let command_re = COMMAND_RE.get_or_init(|| Regex::new("^ *(:[^ ]*)( +(.*))?$").unwrap());
        let mut code_block = CodeBlock::new();
        let mut nodes = Vec::new();

        let mut lines = user_code.lines();
        let mut line_number = 1;
        let mut current_line = lines.next().unwrap_or(user_code);

        for (command_line_offset, line) in user_code.lines().enumerate() {
            // We only accept commands up until the first non-command.
            if let Some(captures) = command_re.captures(line) {
                code_block = code_block.with(
                    CodeKind::Command(CommandCall {
                        command: captures[1].to_owned(),
                        args: captures.get(3).map(|m| m.as_str().to_owned()),
                        start_byte: line.as_ptr() as usize - user_code.as_ptr() as usize,
                        line_number: command_line_offset + 1,
                    }),
                    line,
                );
            } else if line.starts_with(r"//") || line.trim().is_empty() {
                // Ignore blank lines, otherwise we can't have blank lines before :dep commands.
                // We also ignore lines that start with //, because those are line comments.
            } else {
                // Anything else, we treat as Rust code to be executed. Since we don't accept commands after Rust code, we're done looking for commands.
                let non_command_start_byte = line.as_ptr() as usize - user_code.as_ptr() as usize;
                for OriginalUserCode {
                    code,
                    start_byte,
                    node,
                } in
                    statement_splitter::split_into_statements(&user_code[non_command_start_byte..])
                {
                    let node_index = nodes.len();
                    while code.as_ptr() as usize
                        >= current_line.as_ptr() as usize + current_line.len()
                    {
                        line_number += 1;
                        // Unwrap must succeed since code is past the end of the current line.
                        current_line = lines.next().unwrap();
                    }
                    let byte_offset = code.as_ptr() as usize - current_line.as_ptr() as usize;
                    let column_offset = count_columns(&current_line[..byte_offset]);
                    code_block = code_block.with(
                        CodeKind::OriginalUserCode(UserCodeMetadata {
                            start_byte: start_byte + non_command_start_byte,
                            node_index,
                            start_line: line_number,
                            column_offset,
                        }),
                        code,
                    );
                    nodes.push(node);
                }
                break;
            }
        }
        for (index, segment) in code_block.segments.iter_mut().enumerate() {
            segment.sequence = Some(index);
        }
        (
            code_block,
            UserCodeInfo {
                nodes,
                original_lines: user_code.lines().collect(),
            },
        )
    }

    pub(crate) fn command_containing_user_offset(
        &self,
        user_code_offset: usize,
    ) -> Option<(&Segment, usize)> {
        self.segments.iter().find_map(|segment| {
            if let CodeKind::Command(CommandCall { start_byte, .. }) = &segment.kind {
                if user_code_offset >= *start_byte
                    && user_code_offset <= start_byte + segment.code.len()
                {
                    return Some((segment, user_code_offset - *start_byte));
                }
            }
            None
        })
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

    /// Returns the segment type for the specified line (starts from 1) together
    /// with the line offset into that segment. Out-of-range indices will return
    /// type Unknown.
    pub(crate) fn origin_for_line(&self, line_number: usize) -> (&CodeKind, usize) {
        if line_number == 0 {
            return (&CodeKind::Unknown, 0);
        }
        let mut current_line_number = 1;
        for segment in &self.segments {
            if current_line_number + segment.num_lines > line_number {
                return (&segment.kind, line_number - current_line_number);
            }
            current_line_number += segment.num_lines;
        }
        (&CodeKind::Unknown, 0)
    }

    pub(crate) fn apply_fallback(&mut self, fallback: &CodeBlock) {
        let mut replacement_segments = Vec::new();
        for segment in std::mem::take(&mut self.segments) {
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
    use super::CodeBlock;
    use super::CodeKind;

    #[test]
    fn basic_usage() {
        let user_code = "l3";
        let (user_code_block, _nodes) = CodeBlock::from_original_user_code(user_code);
        let mut code = CodeBlock::new()
            .generated("l1\nl2")
            .add_all(user_code_block)
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
        assert_eq!(code.origin_for_line(0), (&CodeKind::Unknown, 0));
        assert_eq!(code.origin_for_line(1), (&CodeKind::OtherGeneratedCode, 0));
        assert_eq!(code.origin_for_line(2), (&CodeKind::OtherGeneratedCode, 1));
        if let (CodeKind::OriginalUserCode(meta3), 0) = code.origin_for_line(3) {
            assert_eq!(meta3.start_byte, 0);
        } else {
            panic!("Unexpected result for line 3");
        }
        assert_eq!(code.origin_for_line(4), (&CodeKind::OtherGeneratedCode, 0));
        assert_eq!(
            code.origin_for_line(5),
            (
                &CodeKind::PackVariable {
                    variable_name: "v".to_owned()
                },
                0
            )
        );
        assert_eq!(code.origin_for_line(6), (&CodeKind::Unknown, 0));

        assert_eq!(
            &code.code_string()[code.user_offset_to_output_offset(0).unwrap()
                ..code.user_offset_to_output_offset(user_code.len()).unwrap()],
            user_code
        );
    }
}
