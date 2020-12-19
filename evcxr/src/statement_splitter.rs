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

use ra_ap_syntax::{ast, AstNode, SourceFile};

/// Information about some code that the user supplied.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct UserCodeMetadata {
    /// The starting byte in the code as the user wrote it.
    pub(crate) start_byte: usize,
}

/// Attempt to split some code into separate statements. All of the input will be returned besides
/// possibly some trailing whitespace. i.e. if we can't parse it as statements, everything from the
/// point where we can't parse onwards will be returned as a single statement.
pub(crate) fn split_into_statements(code: &str) -> Vec<(&str, UserCodeMetadata)> {
    let mut output = Vec::new();
    let prelude = "fn f(){";
    let parsed_file = SourceFile::parse(&(prelude.to_owned() + code));
    let mut start_byte = 0;
    if let Some(block) = parsed_file
        .syntax_node()
        .children()
        .next()
        .and_then(ast::Fn::cast)
        .as_ref()
        .and_then(ast::Fn::body)
    {
        let mut children = block.syntax().children().peekable();
        while let (Some(_child), next) = (children.next(), children.peek()) {
            // With the possible exception of the first node, We want to include
            // whitespace after nodes rather than before, so our end is the
            // start of the next node, or failing that the end of the code.
            let end = next
                .map(|next| usize::from(next.text_range().start()) - prelude.len())
                .unwrap_or(code.len());
            output.push((&code[start_byte..end], UserCodeMetadata { start_byte }));
            start_byte = end;
        }
    }
    output
}

#[cfg(test)]
mod test {
    use super::split_into_statements;

    fn split_and_get_text(code: &str) -> Vec<&str> {
        split_into_statements(code)
            .into_iter()
            .map(|(code, _meta)| code)
            .collect()
    }

    #[test]
    fn single_line() {
        assert_eq!(
            split_and_get_text("let mut a = 10i32; a += 32;"),
            vec!["let mut a = 10i32; ", "a += 32;"]
        );
    }

    #[test]
    fn multiple_lines() {
        assert_eq!(
            split_and_get_text("let mut a = 10i32;\n  foo()"),
            vec!["let mut a = 10i32;\n  ", "foo()"]
        );
    }

    #[test]
    fn statement_ends_with_brace() {
        assert_eq!(
            split_and_get_text("if a == b {foo(); bar();} baz();"),
            vec!["if a == b {foo(); bar();} ", "baz();"]
        );
    }

    #[test]
    fn else_statements() {
        assert_eq!(
            split_and_get_text("if a == b {foo(); bar();}  else if a < b {baz();} else {foo();} b"),
            vec![
                "if a == b {foo(); bar();}  else if a < b {baz();} else {foo();} ",
                "b"
            ]
        );
    }

    #[test]
    fn crate_attributes() {
        assert_eq!(
            split_and_get_text("#![allow(non_ascii_idents)] let a = 10;"),
            vec!["#![allow(non_ascii_idents)] ", "let a = 10;"]
        );
    }

    #[test]
    fn partial_code() {
        let code = r#"
        fn foo() -> Vec<String> {
            vec![]
        }
        foo().res"#;
        assert_eq!(
            split_and_get_text(code),
            vec![
                r#"
        fn foo() -> Vec<String> {
            vec![]
        }
        "#,
                "foo().res"
            ]
        );
    }
}
