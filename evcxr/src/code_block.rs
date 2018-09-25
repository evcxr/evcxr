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

use std;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Segment {
    code_origin: CodeOrigin,
    code: String,
    num_lines: usize,
}

impl Segment {
    fn new(code_origin: CodeOrigin, mut code: String) -> Segment {
        if !code.ends_with('\n') {
            code.push('\n');
        }
        Segment {
            code_origin,
            num_lines: num_lines(&code),
            code,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum CodeOrigin {
    /// The code was supplied by the user. Errors should be reported to the user.
    UserSupplied,
    /// Code is packing a variable into the variable store. Failure modes include (a) incorrect type
    /// (b) variable has been moved (c) non-static lifetime.
    PackVariable { variable_name: String },
    /// Used to check if a variable implements Copy.
    AssertCopyType { variable_name: String },
    /// A line of code that has a fallback to be used in case the supplied line fails to compile.
    WithFallback(CodeBlock),
    /// Code that we generated, but which we don't expect errors from. If we get errors there's not
    /// much we can do besides give the user as much information as we can, appologise and ask to
    /// file a bug report.
    OtherGeneratedCode,
    /// We had trouble determining what the error applied to.
    Unknown,
}

impl CodeOrigin {
    /// Returns whether self is a WithFallback where the replacement is equal to the supplied
    /// fallback. Using the whole fallback as an "ID" may seem a bit heavy handed, but I doubt if
    /// this is likely to ever be a performance consideration. Also, in theory we should pehaps use
    /// the code being replaced as the ID, but in practice the fallback is equally unique.
    fn equals_fallback(&self, fallback: &CodeBlock) -> bool {
        if let CodeOrigin::WithFallback(self_fallback) = self {
            return self_fallback == fallback;
        }
        false
    }
}

fn num_lines(code: &str) -> usize {
    code.chars().filter(|ch| *ch == '\n').count()
}

/// Represents a unit of code to be sent to the compiler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodeBlock {
    segments: Vec<Segment>,
}

impl CodeBlock {
    pub(crate) fn new() -> CodeBlock {
        CodeBlock {
            segments: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub(crate) fn with<T: Into<String>>(mut self, origin: CodeOrigin, code: T) -> Self {
        self.segments.push(Segment::new(origin, code.into()));
        self
    }

    pub(crate) fn code_with_fallback<T: Into<String>>(self, code: T, fallback: CodeBlock) -> Self {
        self.with(CodeOrigin::WithFallback(fallback), code)
    }

    pub(crate) fn generated<T: Into<String>>(self, code: T) -> Self {
        self.with(CodeOrigin::OtherGeneratedCode, code)
    }

    pub(crate) fn user_code<T: Into<String>>(self, code: T) -> Self {
        self.with(CodeOrigin::UserSupplied, code)
    }

    pub(crate) fn load_variable(&mut self, code: String) {
        self.segments
            .push(Segment::new(CodeOrigin::OtherGeneratedCode, code));
    }

    pub(crate) fn pack_variable(&mut self, variable_name: String, code: String) {
        self.segments.push(Segment::new(
            CodeOrigin::PackVariable { variable_name },
            code,
        ));
    }

    pub(crate) fn assert_copy_variable(&mut self, variable_name: String, code: String) {
        self.segments.push(Segment::new(
            CodeOrigin::AssertCopyType { variable_name },
            code,
        ));
    }

    pub(crate) fn add_all(mut self, other: CodeBlock) -> Self {
        self.segments.extend(other.segments);
        self
    }

    pub(crate) fn to_string(&self) -> String {
        let mut output = String::new();
        for segment in &self.segments {
            output.push_str(&segment.code);
        }
        output
    }

    /// Returns the segment type for the specified line (starts from 1). Out-of-range indices will
    /// return type Unknown.
    pub(crate) fn origin_for_line(&self, line_number: usize) -> CodeOrigin {
        if line_number == 0 {
            return CodeOrigin::Unknown;
        }
        let mut current_line_number = 1;
        for segment in &self.segments {
            current_line_number += segment.num_lines;
            if current_line_number > line_number {
                return segment.code_origin.clone();
            }
        }
        CodeOrigin::Unknown
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
            if segment.code_origin.equals_fallback(fallback) {
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
    use super::{CodeBlock, CodeOrigin};

    #[test]
    fn basic_usage() {
        let mut code = CodeBlock::new()
            .generated("l1\nl2")
            .user_code("l3")
            .add_all(CodeBlock::new().generated("l4"));
        code.pack_variable("v".to_owned(), "l5".to_owned());
        assert_eq!(code.to_string(), "l1\nl2\nl3\nl4\nl5\n");
        assert_eq!(code.segments.len(), 4);
        assert_eq!(
            code.segments
                .iter()
                .map(|s| s.num_lines)
                .collect::<Vec<_>>(),
            vec![2, 1, 1, 1]
        );
        assert_eq!(code.origin_for_line(0), CodeOrigin::Unknown);
        assert_eq!(code.origin_for_line(1), CodeOrigin::OtherGeneratedCode);
        assert_eq!(code.origin_for_line(2), CodeOrigin::OtherGeneratedCode);
        assert_eq!(code.origin_for_line(3), CodeOrigin::UserSupplied);
        assert_eq!(code.origin_for_line(4), CodeOrigin::OtherGeneratedCode);
        assert_eq!(
            code.origin_for_line(5),
            CodeOrigin::PackVariable {
                variable_name: "v".to_owned()
            }
        );
        assert_eq!(code.origin_for_line(6), CodeOrigin::Unknown);
    }
}
