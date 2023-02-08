// Copyright 2020 The Evcxr Authors.
//
// Licensed under the Apache License, Version 2.0 <LICENSE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE
// or https://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use ra_ap_syntax::ast;
use ra_ap_syntax::AstNode;
use ra_ap_syntax::SourceFile;
use ra_ap_syntax::SyntaxNode;

pub(crate) struct OriginalUserCode<'a> {
    pub(crate) code: &'a str,
    pub(crate) node: SyntaxNode,
    pub(crate) start_byte: usize,
}

/// Attempt to split some code into separate statements. All of the input will be returned besides
/// possibly some trailing whitespace. i.e. if we can't parse it as statements, everything from the
/// point where we can't parse onwards will be returned as a single statement.
pub(crate) fn split_into_statements(code: &str) -> Vec<OriginalUserCode> {
    let mut output = Vec::new();
    let prelude = "fn f(){";
    let parsed_file = SourceFile::parse(&(prelude.to_owned() + code + "}"));
    let mut start_byte = 0;
    if let Some(stmt_list) = parsed_file
        .syntax_node()
        .children()
        .next()
        .and_then(ast::Fn::cast)
        .as_ref()
        .and_then(ast::Fn::body)
        .as_ref()
        .and_then(ast::BlockExpr::stmt_list)
    {
        let mut children = stmt_list.syntax().children().peekable();
        while let (Some(child), next) = (children.next(), children.peek()) {
            // With the possible exception of the first node, We want to include
            // whitespace after nodes rather than before, so our end is the
            // start of the next node, or failing that the end of the code.
            let end = next
                .map(|next| usize::from(next.text_range().start()) - prelude.len())
                .unwrap_or(code.len());
            output.push(OriginalUserCode {
                code: &code[start_byte..end],
                node: child,
                start_byte,
            });
            start_byte = end;
        }
    }
    output
}

#[cfg(test)]
mod test {
    use ra_ap_syntax::ast;
    use ra_ap_syntax::AstNode;
    use ra_ap_syntax::SyntaxKind;

    use super::split_into_statements;

    fn split_and_get_text(code: &str) -> Vec<&str> {
        split_into_statements(code)
            .into_iter()
            .map(|orig| orig.code)
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

    #[test]
    fn expression() {
        let out = split_into_statements("42");
        assert_eq!(out.len(), 1);
        let out = &out[0];
        assert_eq!(out.node.kind(), SyntaxKind::LITERAL);
    }

    #[test]
    fn macro_call() {
        let out = split_into_statements("foo!(42)");
        assert_eq!(out.len(), 1);
        let out = &out[0];
        assert!(ast::Expr::can_cast(out.node.kind()));
    }

    #[test]
    fn single_line_use_and_expr() {
        let out = split_into_statements("use foo::Bar; Bar::result()");
        assert_eq!(out.len(), 2);
        assert!(ast::Use::can_cast(out[0].node.kind()));
        assert_eq!(out[0].code, "use foo::Bar; ");
        assert!(ast::Expr::can_cast(out[1].node.kind()));
        assert_eq!(out[1].code, "Bar::result()");
    }
}
