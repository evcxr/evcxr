# Evcxr REPL

[![Latest Version](https://img.shields.io/crates/v/evcxr_repl.svg)](https://crates.io/crates/evcxr_repl)

A REPL (Read-Eval-Print loop) for Rust.

## Installation and usage

```sh
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
* Since each line is compiled as a separate crate, impls generally need to go on
  the same line as the type they're for.

## Similar projects

* [cargo-eval](https://github.com/reitermarkus/cargo-eval) Not interactive, but
  it gives you a quick way to evaluate Rust code from the command line and/or
  scripts.
* [rusti](https://github.com/murarth/rusti). From a quick look, it appears to
  require a nightly compiler from 2016 and doesn't appear to persist variable
  values. So I suspect the way that it works is pretty different.
