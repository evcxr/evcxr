// Copyright 2019 Google Inc.
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

//! Sadly, `syn` and friends only handle valid rust -- or at least, `syn` tells
//! us that an input is invalid, but cannot determine the difference between
//! inputs that are invalid due to incompleteness and those that are completely
//! invalid as it stands. (This seems due to the fact that rust's lexical
//! primitive is not the token, but the token *tree*, which guarantees brackets
//! are properly nested.
//!
//! I looked around and short of writing a parser, nothing on crates.io helps.
//! So, this is a minimal scanner that attempts to find if input is obviously
//! unclosed. It handles:
//!
//! - Various kinds of brackets: `()`, `[]`, `{}`.
//!
//! - Strings, including raw strings with preceeding hashmarks (e.g.
//!   `r##"foo"##`)
//!
//! - Comments, including nested block comments. `/*/` is (properly) not treated
//!   as a self-closing comment, but the opening of another nesting level.
//!
//! - char/byte literals, but not very well, as they're confusing with
//!   lifetimes. It does just well enough to know that '{' doesn't open a curly
//!   brace, or to get tripped up by '\u{}'
//!
//! It doesn't handle
//!
//! - Closure arguments like `|x|`.
//!
//! - Generic paremeters like `<` (it's possible we could catch them in the
//!   turbofish case but probably not worth it).
//!
//! - Incomplete expressions/statements which aren't inside some other of a
//!   nesting, e.g. `foo +` is clearly incomplete, but we don't detect it unless
//!   it has parens around it.
//!
//! In general the goal here was to parse enough not to get confused by cases
//! that would lead us to think complete input was incomplete. This requires
//! handling strings, comments, etc, as they are allowed to have a "{" in it
//! which we'd otherwise think keeps the whole line open.
//!
//! Note that from here, it should be possible to use syn to actually parse
//! things, but that's left alone for now.

use std::iter::Peekable;
use std::str::CharIndices;
use unicode_xid::UnicodeXID;

/// Return type for `validate_source_fragment`
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum FragmentValidity {
    /// Note that despite it's name, this really just means "not obviously
    /// invalid". There are many ways the source might still be invalid or
    /// incomplete that we fail to detect, but that's a limitation of the fact
    /// that we don't actually understand the source beyond purely lexical
    /// information.
    Valid,
    /// This generally means that we see a problem, and believe that, as it
    /// currently stands, additional input is not going to fix the problem. For
    /// example, mismatched braces and the like.
    ///
    /// At the moment we just send your input to rustc right away if we see
    /// this, but the UX is a bit awkward here, as it can mean we send the input
    /// off before you expect, but this seems likely to require changes to
    /// rustyline.
    Invalid,
    /// The input seems good enough, but incomplete. There's some sort of
    /// obvious indication the source is incomplete: an unclosed string quote,
    /// some sort of bracket, etc. It's pretty important that we avoid saying
    /// the source is incomplete when it's actually complete (as this would
    /// prevent the user from submitting.
    Incomplete,
}

/// Determine if a piece of source is valid, invalid, or merely incomplete. This
/// is approximate, see the module comment for details. The intent is for
/// - Incomplete to be used to mean "keep a multiline block going"
/// - Valid to mean "finishing a multiline block is allowed"
/// - and Invalid to mean something fuzzy like "wait for the user to finish the
///   current line, then send to rustc and give an error".
///     - Ideally, we'd indicate some kinds of invalidity to the user before
///       submitting -- it can be pretty surprising to be in the middle of a
///       function, add one-too-many closing parens to a nested function call,
///       and have the whole (obviously incomplete) source get sent off to the
///       compiler.
pub fn validate_source_fragment(source: &str) -> FragmentValidity {
    use Bracket::*;
    let mut stack: Vec<Bracket> = vec![];

    let mut input = source.char_indices().peekable();
    while let Some((i, c)) = input.next() {
        match c {
            // Possibly a comment.
            '/' => match input.peek() {
                Some((_, '/')) => {
                    eat_comment_line(&mut input);
                }
                Some((_, '*')) => {
                    input.next();
                    if !eat_comment_block(&mut input) {
                        return FragmentValidity::Incomplete;
                    }
                }
                _ => {}
            },
            '(' => stack.push(Round),
            '[' => stack.push(Square),
            '{' => stack.push(Curly),
            ')' | ']' | '}' => {
                match (stack.pop(), c) {
                    (Some(Round), ')') | (Some(Square), ']') | (Some(Curly), '}') => {
                        // good.
                    }
                    _ => {
                        // Either the bracket stack was empty or mismatched. In
                        // the future, we should distinguish between these, and
                        // for a bracket mismatch, highlight it in the prompt
                        // somehow. I think this will require changes to
                        // `rustyline`, though.
                        return FragmentValidity::Invalid;
                    }
                }
            }
            '\'' => {
                // A character or a lifetime.
                match eat_char(&mut input) {
                    Some(EatCharRes::SawInvalid) => {
                        return FragmentValidity::Invalid;
                    }
                    Some(_) => {
                        // Saw something valid. These two cases are currently
                        // just to verify eat_char behaves as expected in tests
                    }
                    None => {
                        return FragmentValidity::Incomplete;
                    }
                }
            }
            // Start of a string.
            '\"' => {
                if let Some(sane_start) = check_raw_str(source, i) {
                    if !eat_string(&mut input, sane_start) {
                        return FragmentValidity::Incomplete;
                    }
                } else {
                    return FragmentValidity::Invalid;
                }
            }
            _ => {}
        }
    }
    // Seems good to me if we get here!
    if stack.is_empty() {
        FragmentValidity::Valid
    } else {
        FragmentValidity::Incomplete
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
enum StrKind {
    /// Normal string. Closed on first ", but a backslash can escape a single
    /// quote.
    Normal,
    /// Raw string. Closed after we see a " followed by the right num of
    /// backslashes,
    RawStr { hashes: usize },
}

/// `quote_idx` should point at the byte index of the starting double-quote of a
/// string.
///
/// Returns Some(depth) where depth is how many #### the string has (e.g. how
/// after seeing a closing \", do we need to think we're done), or None if
/// something looks dodgy.
///
/// information that we should hopefully know whether or not the string closed,
/// or None if it seems like an invalid string, in which case
fn check_raw_str(s: &str, quote_idx: usize) -> Option<StrKind> {
    use StrKind::*;
    debug_assert_eq!(s.as_bytes()[quote_idx], b'"');
    let sb = s.as_bytes();
    let index_back =
        |idx: usize| -> Option<u8> { quote_idx.checked_sub(idx).and_then(|i| sb.get(i).copied()) };
    match index_back(1) {
        // Raw string, no hashes.
        Some(b'r') => Some(RawStr { hashes: 0 }),
        Some(b'#') => {
            let mut count = 1;
            loop {
                let c = index_back(1 + count);
                match c {
                    Some(b'#') => count += 1,
                    Some(b'r') => break,
                    // Syntax error?
                    _ => return None,
                }
            }
            Some(RawStr { hashes: count })
        }
        _ => Some(Normal),
    }
}

/// Expects to be called after `iter` has consumed the starting \". Returns true
/// if the string was closed.
fn eat_string(iter: &mut Peekable<CharIndices<'_>>, kind: StrKind) -> bool {
    let (closing_hashmarks, escapes_allowed) = match kind {
        StrKind::Normal => (0, true),
        StrKind::RawStr { hashes } => (hashes, false),
    };

    while let Some((_, c)) = iter.next() {
        match c {
            '"' => {
                if closing_hashmarks == 0 {
                    return true;
                }
                let mut seen = 0;
                while let Some((_, '#')) = iter.peek() {
                    iter.next();
                    seen += 1;
                    if seen == closing_hashmarks {
                        return true;
                    }
                }
            }
            '\\' if escapes_allowed => {
                // Consume whatever is next -- but whatever it was doesn't
                // really matter to us.
                iter.next();
            }
            _ => {}
        }
    }
    false
}

/// Expects to be called after `iter` has *fully consumed* the initial `//`.
///
/// Consumes the entire comment, including the `\n` and returns true, or returns
/// false if we exhausted `iter` before finding a `\n`.
fn eat_comment_line<I: Iterator<Item = (usize, char)>>(iter: &mut I) {
    while let Some((_, c)) = iter.next() {
        if c == '\n' {
            break;
        }
    }
}

/// Expects to be called after `iter` has *fully consumed* the initial `/*`
/// already. returns `true` if it scanned a fully valid nesting, and false
/// otherwise.
fn eat_comment_block(iter: &mut Peekable<CharIndices<'_>>) -> bool {
    let mut depth = 1;
    while depth != 0 {
        let c = if let Some(next) = iter.next() {
            next.1
        } else {
            return false;
        };
        match c {
            '/' => {
                if let Some((_, '*')) = iter.peek() {
                    iter.next();
                    depth += 1;
                }
            }
            '*' => {
                if let Some((_, '/')) = iter.peek() {
                    iter.next();
                    depth -= 1;
                }
            }
            _ => {}
        }
    }
    true
}

/// Return value of `eat_char`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum EatCharRes {
    AteChar,
    SawLifetime,
    SawInvalid,
}

/// This is kinda hacky, but with a simple scanner like ours, it's hard to tell
/// lifetimes and char's apart.
///
/// Well, sort of. It's pretty easy to recognize chars in *valid* syntax, but we
/// need to fail gracefully in the case that a user types some invalid syntax.
/// It's not okay if a user types `foo('a ')` and we think the `(` never got
/// closed.
///
/// This function either:
/// - sees a char literal, and advances the iterator past it, returning
///   `Some(AteChar)`.
/// - sees something that could be a lifetime, and returns `Some(SawLifetime)`.
/// - sees something it knows is invalid, and returns `Some(SawInvalid)`.
/// - hits the end of the string, and returns None. This is `None` rather than
///   another `EatCharRes` case so that we can use `?`.
///
/// Note that the caller (`eat_char`) enforces consistent behavior WRT `input`
/// position in the cases where we don't consume a character.
fn do_eat_char(input: &mut Peekable<CharIndices<'_>>) -> Option<EatCharRes> {
    let (_, nextc) = input.next()?;
    if nextc == '\n' || nextc == '\r' || nextc == '\t' {
        // these are illegal inside a char literal, according to
        // https://doc.rust-lang.org/reference/tokens.html#character-literals
        return Some(EatCharRes::SawInvalid);
    }

    if nextc == '\\' {
        // Eating an escape sequence. Eat the character which was escaped.
        // Critically, this might be a single quote, which would confuse the
        // test in the loop below.
        let (_, c) = input.next()?;
        // Chars which are allowed to appear after a backslash in a char escape,
        // according to the same link as above.
        let esc = ['\\', '\'', '"', 'x', 'u', 'n', 't', 'r', '0'];
        if !esc.contains(&c) {
            return Some(EatCharRes::SawInvalid);
        }
        // At this point, we're reasonably confident it's an escaped char literal.
        // Hope for the best, and read until we see a closing quote or something
        // that definitely doesn't belong. This should probably be made smarter,
        // since the actual syntax for the escape sequences is not that bad.
        while let Some((_, c)) = input.next() {
            if c == '\'' {
                return Some(EatCharRes::AteChar);
            }
            // Sanity check for a newline, which probably indicates an unclosed
            // quote.
            if c == '\n' {
                return Some(EatCharRes::SawInvalid);
            }
        }
        // Hit end of string.
        None
    } else {
        // Not an escape sequence
        let could_be_lifetime = UnicodeXID::is_xid_start(nextc);
        // The first char inside the quote was not a backslash, so it's either
        // some sort of lifetime, or a char literal. If it's a char literal, the
        // very *next* thing should be a closing quote...
        let (_, maybe_end) = input.next()?;

        Some(if maybe_end == '\'' {
            EatCharRes::AteChar
        } else if could_be_lifetime {
            // This is needed to defend against cases like `foo('a ')`. We want
            // to catch that this is invalid, because missing the `)` would be
            // bad -- we might think `(` never got closed.
            EatCharRes::SawLifetime
        } else {
            // Couldn't be a lifetime, but we didn't get a closing quote where
            // we needed, so it must be invalid.
            EatCharRes::SawInvalid
        })
    }
}

/// This should be right called after `input` reads a `'`. See `do_eat_char` for
/// the explanation of what it does, this wrapper just exists to ensure that
/// function leaves `input` in a consistent place in cases other than "ate the
/// character" and "hit end of string", by making sure it doesn't modify the
/// iterator except in those cases.
///
/// This is to keep things consistent/testable and avoid having to say "if we
/// didn't scan a char, the iterator is left wherever it was when we became
/// convinced it couldn't be one", and not because correctness current depends
/// on the behavior.
///
/// Worth noting that it's likely this behavior makes no sense for the
/// SawInvalid case -- in the future we probably want to have that advance past
/// the invalid part.
fn eat_char(input: &mut Peekable<CharIndices<'_>>) -> Option<EatCharRes> {
    let mut scratch_input = input.clone();
    let res = do_eat_char(&mut scratch_input);
    if let Some(EatCharRes::AteChar) | None = res {
        *input = scratch_input;
    }
    res
}

#[derive(Copy, Clone, PartialEq)]
enum Bracket {
    Round,
    Square,
    Curly,
}

#[cfg(test)]
mod test {
    use super::*;
    // Left arg is `None` if the nesting is invalid, or Some(remaining_str) if valid.
    // note that for leaving it with no more chars we use "".
    fn block_comment_test(s: &str, remaining: impl Into<Option<&'static str>>) {
        let mut i = s.char_indices().peekable();
        assert!(i.next().unwrap().1 == '/', "{}", s);
        assert!(i.next().unwrap().1 == '*', "{}", s);
        let good = eat_comment_block(&mut i);
        if let Some(want) = remaining.into() {
            let i = i.next().map_or(s.len(), |t| t.0);
            assert_eq!(&s[i..], want, "{}", s);
        } else {
            assert!(!good, "{}", s);
        }
    }
    fn line_comment_test(s: &str, remaining: &str) {
        // It doesn't care about peekable, but we *are* going to give it one, so
        // do so in the test in case somehow it matters.
        let mut i = s.char_indices().peekable();
        eat_comment_line(&mut i);
        let next_idx = i.next().map_or(s.len(), |(i, _)| i);
        assert_eq!(&s[next_idx..], remaining);
    }
    #[test]
    fn test_comment_scan() {
        block_comment_test("/* */", "");
        block_comment_test("/* */ abcd", " abcd");
        block_comment_test("/* /*/ */ */ 123", " 123");
        block_comment_test("/*/", None);
        block_comment_test("/* /* /* */ */", None);
        block_comment_test("/* /* /* */ */ */", "");
        block_comment_test("/* /* /*/ */ */ */", "");

        line_comment_test("// foo\n bar", " bar");
        line_comment_test("// foo", "");
        // Test some degenerate cases, specifically if we ever call it with
        // different args, it should still behave properly.
        line_comment_test("\n", "");
        line_comment_test("/\n", "");
        line_comment_test("/", "");
        line_comment_test("\n bar", " bar");
        line_comment_test("/\n bar", " bar");
    }

    #[test]
    fn test_string_scan() {
        use StrKind::*;
        assert_eq!(check_raw_str(r#" "" "#, 1), Some(Normal));
        assert_eq!(check_raw_str(r#" r#"" "#, 3), Some(RawStr { hashes: 1 }));
        assert_eq!(check_raw_str(r#" r##"" "#, 4), Some(RawStr { hashes: 2 }));
        assert_eq!(check_raw_str(r#""" "#, 0), Some(Normal));
        // Error cases.
        assert_eq!(check_raw_str(r#" ##"" "#, 3), None);
        assert_eq!(check_raw_str(r#"##"" "#, 2), None);
    }

    fn char_scan_test(test: &str, after: &str, res: Option<EatCharRes>) {
        let mut i = test.char_indices().peekable();

        assert_eq!(i.next().unwrap(), (0, '\''));
        let actual = eat_char(&mut i);
        // Make sure we ended up where we said we would.
        let next_idx = i.next().map_or(test.len(), |t| t.0);
        assert_eq!(&test[next_idx..], after);
        assert_eq!(actual, res, "bad result for {}", test);
    }
    #[test]
    fn test_char_scan() {
        use EatCharRes::*;
        char_scan_test("'static", "static", Some(SawLifetime));
        char_scan_test("'s'abc", "abc", Some(AteChar));
        char_scan_test("'a ", "a ", Some(SawLifetime));
        char_scan_test("'\\\\' foo", " foo", Some(AteChar));
        char_scan_test("'\\\'' foo", " foo", Some(AteChar));
        char_scan_test("'\\u{1234}' foo", " foo", Some(AteChar));
        char_scan_test("'ü' abc", " abc", Some(AteChar));
        char_scan_test("'😀'foo", "foo", Some(AteChar));
        char_scan_test("'😀", "", None);

        char_scan_test("'\\\\'", "", Some(AteChar));
        char_scan_test("'\\\''", "", Some(AteChar));
        char_scan_test("'\\u{1234}", "", None);
        char_scan_test("'\\n'", "", Some(AteChar));

        char_scan_test("'a", "", None);
        char_scan_test("'", "", None);
        char_scan_test("'\n'", "\n'", Some(SawInvalid));
        char_scan_test("') ", ") ", Some(SawInvalid));
        char_scan_test("'\\\n ", "\\\n ", Some(SawInvalid));
        char_scan_test("'\\u{1234} \n\n'", "\\u{1234} \n\n'", Some(SawInvalid));
    }

    fn test_validity(frag: &str, expect: FragmentValidity) {
        assert_eq!(
            validate_source_fragment(frag),
            expect,
            "for source fragment: `{}`",
            frag
        );
        if expect == FragmentValidity::Invalid {
            return;
        }
        // Ensure that for valid/incomplete source strings, all prefixes are
        // either valid or incomplete. It seems like in the finished version of
        // the rustyline validation features we don't get called except on
        // newlines, so this is a bit more aggressive than we actually need.
        // Still, at the moment it doesn't hurt.
        for (i, _) in frag.char_indices() {
            assert_ne!(
                validate_source_fragment(&frag[..i]),
                FragmentValidity::Invalid,
                "validating {:?}, as substring of:\n`{}`",
                &frag[..i],
                frag,
            );
        }
    }
    #[test]
    fn test_valid_source() {
        let valid = |f: &str| {
            test_validity(f, FragmentValidity::Valid);
        };
        let partial = |f: &str| {
            test_validity(f, FragmentValidity::Incomplete);
        };
        let invalid = |f: &str| {
            test_validity(f, FragmentValidity::Invalid);
        };
        valid("let valid = |f: &str| { test_validity(f, FragmentValidity::Valid); };");
        valid(stringify! {
            foo<'static>('\'', 1, r#"##"#);
        });
        invalid("[test)");
        invalid("test)");
        invalid("'['test]");
        partial("fn test_valid_source() {");

        partial("\"test 123");
        partial("r#\"test 123\"");

        valid("r##\"test 123\"# \"##.len()");

        valid("// 123 /*");
        valid("/* 123 /*\n// */ */");
        // Valid, as 'a might start a lifetime
        valid("'a\n");
        // Invalid, as '3 could not.
        invalid("'3\n");
        // This is invalid, but the important thing is that we don't say
        // incomplete.
        invalid("foo('a ')\n");
    }
}
