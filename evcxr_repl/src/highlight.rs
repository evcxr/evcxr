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

use crate::scan::Bracket;

#[derive(Copy, Clone, PartialEq)]
pub(crate) enum Kind {
    Whitespace,
    Ignored,
    Error,
    Comment,
    Ident,
    // keyword, known builtin type, or something else we want to highlight.
    SpecialIdent,
    LitNum,
    LitStr,
    Open(Bracket),
    Close(Bracket),
    Lifetime,
}

fn is_special_ident(s: &str) -> bool {
    match s {
        "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "dyn" | "else"
        | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop"
        | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "self" | "Self"
        | "static" | "struct" | "super" | "trait" | "true" | "type" | "union" | "unsafe"
        | "use" | "where" | "while" => true,
        "_" | "bool" | "str" | "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32"
        | "i64" | "i128" | "isize" => true,
        _ => false,
    }
}

fn get_kind(rustc_token_kind: &rustc_lexer::TokenKind, token_text: &str) -> (Kind, bool) {
    use rustc_lexer::{LiteralKind as LK, RawStrError, TokenKind as TK};
    match rustc_token_kind {
        TK::LineComment { .. } => (Kind::Comment, false),
        TK::BlockComment { terminated, .. } => (Kind::Comment, *terminated),
        TK::Whitespace => (Kind::Whitespace, false),
        TK::Ident => {
            if is_special_ident(token_text) {
                (Kind::SpecialIdent, false)
            } else {
                (Kind::Ident, false)
            }
        }
        TK::RawIdent => (Kind::Ident, false),

        TK::Literal {
            kind: LK::Int { .. },
            ..
        }
        | TK::Literal {
            kind: LK::Float { .. },
            ..
        } => (Kind::LitNum, false),

        TK::Literal {
            kind: LK::Str { terminated, .. },
            ..
        }
        | TK::Literal {
            kind: LK::Char { terminated, .. },
            ..
        }
        | TK::Literal {
            kind: LK::Byte { terminated, .. },
            ..
        }
        | TK::Literal {
            kind: LK::ByteStr { terminated, .. },
            ..
        } => (Kind::LitStr, *terminated),
        TK::Literal {
            kind: LK::RawStr { err, .. },
            ..
        }
        | TK::Literal {
            kind: LK::RawByteStr { err, .. },
            ..
        } => {
            let not_terminated = matches!(err, Some(RawStrError::NoTerminator { .. }));
            (Kind::LitStr, !not_terminated)
        }

        TK::Lifetime { starts_with_number } => (Kind::Lifetime, !*starts_with_number),

        TK::Unknown => (Kind::Error, false),
        TK::OpenParen => (Kind::Open(Bracket::Round), true),
        TK::CloseParen => (Kind::Close(Bracket::Round), true),
        TK::OpenBrace => (Kind::Open(Bracket::Curly), true),
        TK::CloseBrace => (Kind::Close(Bracket::Curly), true),
        TK::OpenBracket => (Kind::Open(Bracket::Square), true),
        TK::CloseBracket => (Kind::Close(Bracket::Square), true),
        _ => (Kind::Ignored, true),
    }
}

pub(crate) struct HighlightToken {
    pub range: std::ops::Range<usize>,
    pub show_err: bool,
    pub paren_mate_idx: Option<usize>,
    pub kind: Kind,
}

pub(crate) fn highlight_lex(s: &str) -> Vec<HighlightToken> {
    if s.is_empty() {
        return vec![];
    }
    let mut pos = 0;
    let mut paren_stack = vec![];
    let mut result: Vec<HighlightToken> = vec![];
    for tok in rustc_lexer::tokenize(s) {
        let range = pos..(pos + tok.len);
        let (kind, mut valid) = get_kind(&tok.kind, &s[range.clone()]);

        let paren_mate_idx = match kind {
            Kind::Open(p) => {
                paren_stack.push((p, result.len()));
                // Filled in later
                None
            }
            Kind::Close(p) => match paren_stack.pop() {
                Some((opened, idx)) => {
                    valid = opened == p;
                    // The index where we'll be inserted.
                    let index = result.len();
                    result[idx].paren_mate_idx = Some(index);
                    result[idx].show_err = !valid;
                    Some(idx)
                }
                None => {
                    valid = false;
                    None
                }
            },
            _ => None,
        };
        result.push(HighlightToken {
            range,
            kind,
            show_err: !valid || kind == Kind::Error,
            paren_mate_idx,
        });

        pos += tok.len;
    }
    if pos < s.len() {
        result.push(HighlightToken {
            range: pos..s.len(),
            kind: Kind::Ignored,
            show_err: false,
            paren_mate_idx: None,
        })
    }
    for (_, idx) in paren_stack {
        result[idx].show_err = true;
    }
    result
}

pub(crate) fn highlight(input: &str, pos: Option<usize>) -> String {
    use std::fmt::Write;
    if input.is_empty() {
        return "".into();
    }

    let parsed = highlight_lex(input);
    let mut mark_idx = None;
    if let Some(p) = pos {
        for tok in &parsed {
            if let Kind::Open(_) | Kind::Close(_) = tok.kind {
                if tok.range.start == p || tok.range.end == p {
                    mark_idx = tok.paren_mate_idx;
                    break;
                }
            }
        }
    }

    let mut res = String::with_capacity(input.len());
    let mut cur = Style::default();
    for (i, tok) in parsed.iter().enumerate() {
        let mut s = get_style(&tok, cur);
        if mark_idx == Some(i) {
            s.reverse = true;
        }
        if (cur.bold && !s.bold)
            || (cur.reverse && !s.reverse)
            || (cur.italic && !s.italic)
            || (cur.color.is_some() && s.color.is_none())
        {
            // In theory there are ways to disable these individually, but to
            // properly support them you need to parse terminfo (or detect via
            // environment variables in some cases), which is a pain -- just
            // emit SGR0 if we turn anything off.
            res.push_str("\x1b[0m");
            cur = Style::default();
        }
        if cur.color != s.color {
            if let Some(c) = s.color {
                write!(res, "\x1b[38;5;{}m", c).unwrap();
                cur.color = s.color;
            }
        }
        if s.bold && !cur.bold {
            res.push_str("\x1b[1m");
            cur.bold = true;
        }
        if s.reverse && !cur.reverse {
            res.push_str("\x1b[7m");
            cur.reverse = true;
        }
        if s.italic && !cur.italic {
            res.push_str("\x1b[3m");
            cur.italic = true;
        }
        res.push_str(&input[tok.range.clone()]);
    }
    res.push_str("\x1b[0m");

    res
}

#[derive(Copy, Clone, Default, PartialEq)]
pub struct Style {
    // Looks a lot better on dark/light screens with 256 color, and is still
    // almost universally supported in practice. Sadly, `colored` doesn't
    // support anything beyond 4-bit color...
    pub color: Option<u8>,
    pub bold: bool,
    pub reverse: bool,
    pub italic: bool,
}

fn get_style(t: &HighlightToken, last: Style) -> Style {
    use Kind::*;
    match t.kind {
        Whitespace => Style {
            reverse: false,
            ..last
        },
        Lifetime | Ident | Ignored => Style::default(),
        Error => Style {
            color: Some(160),
            bold: true,
            ..Default::default()
        },
        Comment => Style {
            color: Some(243),
            italic: true,
            ..Default::default()
        },
        SpecialIdent => Style {
            color: None,
            bold: true,
            ..Default::default()
        },
        LitNum => Style {
            color: Some(132),
            ..Default::default()
        },
        LitStr => Style {
            color: if t.show_err { Some(160) } else { Some(78) },
            ..Default::default()
        },
        Open(_) | Close(_) => Style {
            color: if t.show_err { Some(160) } else { Some(39) },
            bold: t.show_err,
            ..Default::default()
        },
    }
}
