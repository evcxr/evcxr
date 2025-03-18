# Evcxr Rust REPL

[![Latest Version](https://img.shields.io/crates/v/evcxr_repl.svg)](https://crates.io/crates/evcxr_repl)
[![Downloads](https://img.shields.io/crates/d/evcxr_repl)](https://crates.io/crates/evcxr_repl)
[![License](https://img.shields.io/crates/l/evcxr_repl)](https://crates.io/crates/evcxr_repl)

A Rust REPL (Read-Eval-Print loop) built using the
[`evcxr`](https://github.com/evcxr/evcxr/blob/main/evcxr/README.md) evaluation context.

## Installation

Before you can use the REPL, you must download a local copy of Rust's source code:

```sh
$ rustup component add rust-src
```

Once that's done, you have a choice of using pre-built binaries, or building from source.

### Pre-built binaries

Pre-built binaries are available from the [Releases](https://github.com/evcxr/evcxr/releases) page.
These are currently built for Linux, Mac and Windows. In each case they are built with the latest
version of each operating system. In the case of Linux, they're built on the latest Ubuntu. If your
version of libc is different, they might not work, in which case you'll need to build yourself. See
below.

### Building yourself

If you can't, or don't want to use the pre-built binaries, you can build from
source.

Make sure you've got a recent version of rust installed. Evcxr's dependencies often make use of new
Rust features shortly after they're stabilized, so it's not uncommon that the latest release of
Evcxr will end up requiring the latest version of rustc.

Now you can go ahead and install the binary:
```
$ cargo install --locked evcxr_repl
```

## Usage

And start the REPL:
```sh
$ evcxr  
Welcome to evcxr. For help, type :help
>> 
```

## Completion Type

Evcxr supports two modes of tab completion:

* List: When you press tab, it will complete any common prefix shared by all available completions.
  Pressing tab twice will then list all available completions. This mode is the default.
* Circular: When you press tab, it will show the first completion. Pressing tab again will cycle
  through all the available completions, then return to the start. To select this mode, set the
  environment variable EVCXR_COMPLETION_TYPE=circular.

## Usage information

Evcxr is both a REPL and a Jupyter kernel. See [Evcxr common
usage](https://github.com/evcxr/evcxr/blob/main/COMMON.md) for usage information that is common to
both.

## Manual Installation

You can install the REPL manually with git:

```sh
$ cargo install --force --git https://github.com/evcxr/evcxr.git evcxr_repl
```

## Similar projects

* [irust](https://crates.io/crates/irust). Looks to have quite a sophisticated command line
  interface. All your code gets rerun each time. For many uses this is fine and has the advantage
  that you can have variables reference other variables.
* [cargo-eval](https://github.com/reitermarkus/cargo-eval) Not interactive, but it gives you a quick
  way to evaluate Rust code from the command line and/or scripts.
* [rusti](https://github.com/murarth/rusti). Deprecated since 2019. Also, rusti requires a nightly
  compiler from 2016 and doesn't appear to persist variable values.
* [Papyrus](https://github.com/kurtlawrence/papyrus). Looks like it's no longer maintained.
