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
    Ident(IdentKind),
    // keyword, known builtin type, or something else we want to highlight.
    // SpecialIdent,
    Lit(LitKind),
    Open(Bracket),
    Close(Bracket),
    // Punct,
    Lifetime,
}

#[derive(Copy, Clone, PartialEq)]
pub(crate) enum IdentKind {
    Normal,
    Keyword,
    PrimTy,
    Reserved,
}

#[derive(Copy, Clone, PartialEq)]
pub(crate) enum LitKind {
    // true or false
    Bool,
    // int or float
    Num,
    // (possibly raw) string or byte string
    Str,
    // char or byte lit
    Char,
}

fn kind_for_ident(s: &str) -> Kind {
    match s {
        "as" | "async" | "await" | "box" | "break" | "const" | "continue" | "crate" | "dyn"
        | "else" | "enum" | "existential" | "extern" | "fn" | "for" | "if" | "impl" | "in"
        | "let" | "loop" | "macro" | "match" | "mod" | "move" | "mut" | "pub" | "raw" | "ref"
        | "return" | "self" | "Self" | "static" | "struct" | "super" | "trait" | "try" | "type"
        | "union" | "unsafe" | "use" | "where" | "while" | "yield" | "_" => {
            Kind::Ident(IdentKind::Keyword)
        }

        "true" | "false" => Kind::Lit(LitKind::Bool),

        "bool" | "str" | "char" | "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32"
        | "i64" | "i128" | "isize" | "f32" | "f64" => Kind::Ident(IdentKind::PrimTy),

        // reserved words that don't even work with unstable features — marked
        // in red in a kind of "error" way.
        "abstract" | "become" | "do" | "final" | "override" | "typeof" | "unsized" | "virtual" => {
            Kind::Ident(IdentKind::Reserved)
        }

        _ => Kind::Ident(IdentKind::Normal),
    }
}

// Returns `(Kind, is_complete)`
fn get_kind(rustc_token_kind: &rustc_lexer::TokenKind, token_text: &str) -> (Kind, bool) {
    use rustc_lexer::{LiteralKind as LK, RawStrError, TokenKind as TK};
    match rustc_token_kind {
        TK::LineComment { .. } => (Kind::Comment, false),
        TK::BlockComment { terminated, .. } => (Kind::Comment, *terminated),
        TK::Whitespace => (Kind::Whitespace, false),

        TK::Ident => (kind_for_ident(token_text), false),
        TK::RawIdent => (Kind::Ident(IdentKind::Normal), false),

        // number lit
        TK::Literal {
            kind: LK::Int { .. },
            ..
        }
        | TK::Literal {
            kind: LK::Float { .. },
            ..
        } => (Kind::Lit(LitKind::Num), false),
        // char/byte lit
        TK::Literal {
            kind: LK::Char { terminated, .. },
            ..
        }
        | TK::Literal {
            kind: LK::Byte { terminated, .. },
            ..
        } => (Kind::Lit(LitKind::Char), *terminated),
        // non-raw string lit
        TK::Literal {
            kind: LK::Str { terminated, .. },
            ..
        }
        | TK::Literal {
            kind: LK::ByteStr { terminated, .. },
            ..
        } => (Kind::Lit(LitKind::Str), *terminated),
        // raw string lit
        TK::Literal {
            kind: LK::RawStr { err, .. },
            ..
        }
        | TK::Literal {
            kind: LK::RawByteStr { err, .. },
            ..
        } => {
            let not_terminated = matches!(err, Some(RawStrError::NoTerminator { .. }));
            (Kind::Lit(LitKind::Str), !not_terminated)
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
impl HighlightToken {
    fn err_or(&self, err_color: u8, good_color: u8) -> Option<u8> {
        if self.show_err {
            Some(err_color)
        } else {
            Some(good_color)
        }
    }
    fn text<'a>(&self, input: &'a str) -> &'a str {
        &input[self.range.clone()]
    }
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
        for (pos, tok) in parsed.iter().enumerate() {
            // Ideally, this would just check that both these are met:
            //
            // - tok.kind is open or closed
            // - tok.range.start or tok.range.end is p
            //
            // unfortunately, there's an edge-case or bug of some sort in
            // rustyline's logic we hit if we do this.
            //
            // Basically:
            //
            // - if the closing delimiter is the last thing in the input
            //   except for possibly trailing whitespace starting with a newline,
            // - and the cursor is at range.end for the closing delimiter
            //
            // then when the user submits the line, the matched delimiter will
            // not unhighlight, which looks jank as heck.
            //
            // This appears to actually be either deliberate in rustyline, or
            // an artifact of the current API for this, so... we work around.
            let at_end = pos == parsed.len() - 1
                || (pos == parsed.len() - 2
                    && (parsed[parsed.len() - 1].kind == Kind::Whitespace
                        && parsed[parsed.len() - 1].text(input).starts_with('\n')));

            let is_match = (matches!(tok.kind, Kind::Close(_))
                && (tok.range.start == p || (tok.range.end == p && !at_end)))
                || (matches!(tok.kind, Kind::Open(_))
                    && (tok.range.end == p || tok.range.start == p));

            if is_match {
                mark_idx = tok.paren_mate_idx;
                break;
            }
        }
    }
    let cap_guess = (input.len() as f32 * 1.5) as usize;
    let mut res = String::with_capacity(cap_guess);
    let mut cur = Style::default();
    for (i, tok) in parsed.iter().enumerate() {
        // let pos = res.len();
        let mut s = get_style(&tok, cur);
        if mark_idx == Some(i) {
            s.bold = true;
            // s.reverse = true;
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
            res.push_str("\x1b[m");
            cur = Style::default();
        }
        if s.reverse && !cur.reverse {
            res.push_str("\x1b[7m");
            cur.reverse = true;
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
        if s.italic && !cur.italic {
            res.push_str("\x1b[3m");
            cur.italic = true;
        }
        res.push_str(tok.text(input))
    }

    // I'm... completely unsure why this is needed 😿
    if res.ends_with('\n') {
        res.pop();
    }

    res.push_str("\x1b[m");

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
        Ident(IdentKind::Normal) | Ignored | Lifetime => Style::default(),
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
        Ident(IdentKind::Reserved) => Style {
            color: Some(160),
            ..Default::default()
        },
        Ident(IdentKind::Keyword) | Lit(LitKind::Bool) => Style {
            color: None,
            bold: true,
            ..Default::default()
        },
        Ident(IdentKind::PrimTy) => Style {
            color: None,
            bold: true,
            ..Default::default()
        },
        Lit(LitKind::Num) => Style {
            color: Some(132),
            ..Default::default()
        },
        Lit(LitKind::Str) => Style {
            color: t.err_or(160, 78),
            ..Default::default()
        },
        Lit(LitKind::Char) => Style {
            color: t.err_or(160, 78),
            ..Default::default()
        },
        Open(_) | Close(_) => Style {
            color: t.err_or(160, 39),
            bold: t.show_err,
            ..Default::default()
        },
    }
}
