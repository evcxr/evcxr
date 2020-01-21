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

use regex::Regex;
use syn;

/// Attempt to split some code into separate statements. All of the input will be returned besides
/// possibly some trailing whitespace. i.e. if we can't parse it as statements, everything from the
/// point where we can't parse onwards will be returned as a single statement.
pub fn split_into_statements(mut code: &str) -> Vec<&str> {
    // Once proc_macro2::Span can be used, we shouldn't need this - we can instead just put
    // everything inside braces and parse it as a block, then get the spans for whichever bits we
    // care about. Until then, we have to live with this.

    lazy_static! {
        static ref SEPARATOR: Regex = Regex::new("(;|};?) *\n?").unwrap();
    }
    lazy_static! {
        static ref ELSE_AT_START: Regex = Regex::new("^[ \n]*else[ \n]").unwrap();
    }

    let mut output = Vec::new();
    loop {
        let mut got_statment = false;
        for m in SEPARATOR.find_iter(code) {
            let offset = m.end();
            let candidate = &code[..offset];
            let remainder = &code[offset..];
            if ELSE_AT_START.is_match(remainder) {
                continue;
            }
            if syn::parse_str::<syn::Stmt>(candidate).is_ok() {
                output.push(candidate);
                code = remainder;
                got_statment = true;
                break;
            }
        }
        if !got_statment {
            break;
        }
    }
    // Whatever is left gets added, whether or not it parses as a statement. That way errors can be
    // reported.
    if !code.trim().is_empty() {
        output.push(code);
    }
    output
}

#[cfg(test)]
mod test {
    use super::split_into_statements;

    #[test]
    fn single_line() {
        assert_eq!(
            split_into_statements("let mut a = 10i32; a += 32;"),
            vec!["let mut a = 10i32; ", "a += 32;"]
        );
    }

    #[test]
    fn multiple_lines() {
        assert_eq!(
            split_into_statements("let mut a = 10i32;\n  foo()"),
            vec!["let mut a = 10i32;\n", "  foo()"]
        );
    }

    #[test]
    fn statement_ends_with_brace() {
        assert_eq!(
            split_into_statements("if a == b {foo(); bar();} baz();"),
            vec!["if a == b {foo(); bar();} ", "baz();"]
        );
    }

    #[test]
    fn else_statements() {
        assert_eq!(
            split_into_statements(
                "if a == b {foo(); bar();}  else if a < b {baz();} else {foo();} b"
            ),
            vec![
                "if a == b {foo(); bar();}  else if a < b {baz();} else {foo();} ",
                "b"
            ]
        );
    }
}
