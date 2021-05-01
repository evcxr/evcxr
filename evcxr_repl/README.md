# Evcxr Rust REPL

[![Latest Version](https://img.shields.io/crates/v/evcxr_repl.svg)](https://crates.io/crates/evcxr_repl)
[![Downloads](https://img.shields.io/crates/d/evcxr_repl)](https://crates.io/crates/evcxr_repl)
[![License](https://img.shields.io/crates/l/evcxr_repl)](https://crates.io/crates/evcxr_repl)

A Rust REPL (Read-Eval-Print loop) built using the [`evcxr`](https://github.com/google/evcxr/blob/main/evcxr/README.md) evaluation context.

## Installation and Usage

Before you install the REPL, you must download a local copy of Rust's source code:
```sh
$ rustup component add rust-src
```

Now you can go ahead and install the binary:
```
$ cargo install evcxr_repl
```

And start the REPL:
```sh
$ evcxr  
Welcome to evcxr. For help, type :help
>> 
```

## Usage information

Evcxr is both a REPL and a Jupyter kernel. See [Evcxr common
usage](https://github.com/google/evcxr/blob/main/COMMON.md) for usage information that is
common to both.

## Manual Installation

You can install the REPL manually with git:

```sh
$ cargo install --force --git https://github.com/google/evcxr.git evcxr_repl
```

## Similar projects

* [irust](https://crates.io/crates/irust). Looks to have quite a sophisticated command line interface. If you don't need variable preservation, this is probably worth checking out.
* [cargo-eval](https://github.com/reitermarkus/cargo-eval) Not interactive, but it gives you a quick way to evaluate Rust code from the command line and/or scripts.
* [rusti](https://github.com/murarth/rusti). Deprecated since 2019. Also, rusti requires a nightly compiler from 2016 and doesn't appear to persist variable values.
* [Papyrus](https://github.com/kurtlawrence/papyrus). Looks like it's no longer maintained.
