# Evcxr REPL

[![Latest Version](https://img.shields.io/crates/v/evcxr_repl.svg)](https://crates.io/crates/evcxr_repl)
[![Downloads](https://img.shields.io/crates/d/evcxr_repl)](https://crates.io/crates/evcxr_repl)
[![License](https://img.shields.io/crates/l/evcxr_repl)](https://crates.io/crates/evcxr_repl)

A REPL (Read-Eval-Print loop) for Rust using the [`evcxr`](https://github.com/google/evcxr/blob/master/evcxr/README.md) evaluation context.

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

## Features

**Define functions, structs, enums, and other data types:**
```rust
>> pub struct User {
     username: String
  }
>> let user = User { username: String::from("John Doe") };
```


**Assign values to variables then make use of them later:**
```rust
>> let x = "hello";
>> // do other things
>> println!("{}", x)
```

**Load crates from [crates.io](https://crates.io/):**
```rust
>> :dep rand = { version = "0.7.3" }
>> let x: u8 = rand::random();
```
*Note that loading large crates with many dependencies may take a while.*

**Nice error reporting:**
```sh
>> let x = unknown();
           ^^^^^^ not found in this scope
cannot find function `unknown` in this scope
help: consider importing this function
```

## Limitations

* Storing references into variables that persist between compilations is not permitted.
* There is currently no way to import macros from external crates.

## Documentation

### Startup Options

You can create an `init.evcxr` in one of the following locations, depending on your operating system:

| Linux                        | OSX                                                 | Windows                                           |
|------------------------------|-----------------------------------------------------|---------------------------------------------------|
|` ~/.config/evcxr/init.evcxr` | `/Users/Alice/Library/Preferences/evcxr/init.evcxr` | `C:\Users\Alice\AppData\Roaming\evcxr\init.evcxr` |

Any options set in this file will be automatically loaded at startup. For example:

```rust
:timing
:dep { rand = "0.7.3" }
:dep { log = "0.4.11" }
```

### Caching

You can optionally cache compilation outputs with [scacche](https://github.com/mozilla/sccache). If you frequently use the same crates, this can speed things up quite a bit.

You can install scacche with cargo:
```sh
$ cargo install sccache
```

And set the scacche configuration option:
```sh
:sccache 1
```

### Variable Persistence

The `:vars` command will list all the variables defined in the current context:
```rust
>> let x = 0;
>> let y = 1;
>> :vars
y: i32
x: i32
```

If your code panics, all variables will be lost. To preserve variables on panics, you can set the `:preserve_vars_on_panic` configuration option:
```rust
>> :preserve_vars_on_panic 1
Preserve vars on panic: true
```

Only variables that either are not referenced by the code being run or implement `Copy` will be preserved. Also note that this will slow down compilation.

### Linker

Installing the [`lld`](https://lld.llvm.org/) linker it is recommended as it is generally faster than the default system linker. On Debian-based systems you might be able to install it with:
```sh
$ sudo apt install lld
```
`lld` will be used automatically if detected on all systems with the exception of Mac OS. You can check which linker is being used by running `:linker`.

### Commands

Here is a complete list of the configuration options you can set to customize your REPL experience:

* `:opt [level]`      Toggle/set optimization level
* `:fmt [format]`     Set output formatter (default: `{:?}`). 
* `:efmt [format]`    Set the formatter for errors returned by `?`
* `:sccache [0|1]`    Set whether to use sccache.
* `:linker [linker]`  Set/print linker. Supported: `system`, `lld`
* `:timing`           Toggle printing of how long evaluations take
* `:time_passes`      Toggle printing of rustc pass times (requires nightly)
* `:internal_debug`   Toggle internal code debugging output
* `:preserve_vars_on_panic [0|1]`  Try to keep vars on panic

And here are the supported REPL commands:

* `:explain`          Print the explanation of last error
* `:clear`            Clear all state, keeping compilation cache
* `:last_compile_dir` Print the directory in which we last compiled
* `:last_error_json`  Print the last compilation error as JSON (for debugging)
* `:dep`              Add an external dependency. e.g. `:dep regex = "1.0"`
* `:help`             View the help message

## Manual Installation

You can install the REPL manually with git:

```sh
$ cargo install --force --git https://github.com/google/evcxr.git evcxr_repl
```

## Similar projects

* [cargo-eval](https://github.com/reitermarkus/cargo-eval) Not interactive, but it gives you a quick way to evaluate Rust code from the command line and/or scripts.
* [rusti](https://github.com/murarth/rusti). Deprecated since 2019. Also, rusti requires a nightly compiler from 2016 and doesn't appear to persist variable values.
