# Evcxr library

[![Latest Version](https://img.shields.io/crates/v/evcxr.svg)](https://crates.io/crates/evcxr)

An implementation of eval() for Rust.

The main struct in this crate is ```EvalContext```. You create one, then ask it
to eval bits of code. Any defined functions, variables etc are local to that
context.

To see an example usage, see
[examples/example_eval.rs](examples/example_eval.rs).

I'll not go into too much detail here, since the purpose of this library is
really to provide functionality to evcxr\_jupyter and evcxr\_repl. If you'd like
to try using this crate for something else, drop me an email, or file an issue
on the repository and we can figure out your use case.

## MSRV (Minimum supported Rust version)

Should work with the latest stable release. May sometimes work with one or two
older version. See evcxr/Cargo.toml for the current minimum supported rust
version. If you're stuck on an older version of rustc, your best bet is either
to use a precompiled binary or use an older version of Evcxr. The release notes
might give clues as to when MSRV bumps occurred.

## How it works

See [how it works](HOW_IT_WORKS.md)

## Release notes

See [release notes](RELEASE_NOTES.md)
