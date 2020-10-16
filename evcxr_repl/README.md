# Evcxr REPL

[![Latest Version](https://img.shields.io/crates/v/evcxr_repl.svg)](https://crates.io/crates/evcxr_repl)

A REPL (Read-Eval-Print loop) for Rust.

## Installation and usage

```sh
rustup component add rust-src
cargo install evcxr_repl
```

Then run with:

```sh
evcxr
```

## Features

* Define functions, structs, enums etc.
* Assign values to variables then make use of them later.
* Load crates from crates.io.
  * e.g. `:dep regex = { version = "1.0" }` will load the regex crate.
  * This can take a while, especially for large crates with many dependencies.
* Expressions will be debug printed.
* For the most part compilation errors are reported in a reasonably intuitive inline way.

## Limitations

* Storing references into variables that persist between compilations is not permitted.
* Not yet any way to import macros from external crates.

## More documentation

Some of the documentation for [Evcxr
Jupyter](https://github.com/google/evcxr/tree/master/evcxr_jupyter) applies to
the REPL as well. In particular, the later sections such as startup options,
sccache integration and lld.

## Installing from git head

If there's a bugfix in git that you'd like to try out, you can install directly
from git with the command:

```sh
cargo install --force --git https://github.com/google/evcxr.git evcxr_repl
```

## Similar projects

* [cargo-eval](https://github.com/reitermarkus/cargo-eval) Not interactive, but
  it gives you a quick way to evaluate Rust code from the command line and/or
  scripts.
* [rusti](https://github.com/murarth/rusti). From a quick look, it appears to
  require a nightly compiler from 2016 and doesn't appear to persist variable
  values. So I suspect the way that it works is pretty different.
