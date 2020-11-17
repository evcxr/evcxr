# Evcxr common usage information

This page contains usage information that is common to both the [Evcxr REPL](evcxr_repl/README.md)
and the [Evcxr Jupyter kernel](evcxr_jupyter/README.md).

## Usage notes

* If your code panics, all variables will be lost. You can optionally run
  `:preserve_vars_on_panic 1` to turn on preservation of variables. However note
  that this will slow down compilation. Also, only variables that either are not
  referenced by the code being run, or are Copy will be preserved.
* If your code segfaults (e.g. due to buggy unsafe code), aborts, exits etc, the
  process in which the code runs will be restarted. All variables will be lost.

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
```rust
>> let x = unknown();
           ^^^^^^ not found in this scope
cannot find function `unknown` in this scope
help: consider importing this function
```

**Support for the `?` operator**

Evcxr will automatically propagate errors caught with the `?` operator to the formatter set with `:efmt`. For example:
```rust
>> let var = std::env::var("UNKNOWN")?;
NotPresent
```

If `:efmt` is set to `Debug`, this will also work for `Option`:
```rust
>> [1, 2, 3].get(4)?
NoneError
```

## Limitations

* Storing references into variables that persist between compilations is not permitted.
* There is currently no way to import macros from external crates.

## Documentation

### Startup

You can create an `init.evcxr` file in the `evcxr` config directory. The location of this directory varies depending on your operating system:

| Linux             | OSX                                      | Windows                                |
|-------------------|------------------------------------------|----------------------------------------|
|` ~/.config/evcxr` | `/Users/Alice/Library/Preferences/evcxr` | `C:\Users\Alice\AppData\Roaming\evcxr` |

Any options set in this file will be automatically loaded at startup. For example:

```rust
:timing
:dep { rand = "0.7.3" }
:dep { log = "0.4.11" }
```

You can also create an `prelude.rs` file which will be evaluated on startup. For example:
```rust
// prelude.rs
const msg: &str = "hello";
```

```rust
$ evcxr                                                   
Welcome to evcxr. For help, type :help
Executing prelude from "~/.config/evcxr/prelude.rs"
>> msg
"hello"
```

### Caching

You can optionally cache compilation outputs with [sccache](https://github.com/mozilla/sccache). If
you frequently use the same crates, this can speed things up quite a bit.

You can install sccache with cargo:
```sh
$ cargo install sccache
```

And set the sccache configuration option:
```sh
:sccache 1
```

To always use sccache, add `:sccache 1` to your init.evcxr (see Startup options above).

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

Here is a complete list of the configuration options you can set to customize your Evcxr experience:

* `:opt [level]`      Toggle/set optimization level
* `:fmt [format]`     Set output formatter (default: `{:?}`). 
* `:efmt [format]`    Set the formatter for errors returned by `?`
* `:sccache [0|1]`    Set whether to use sccache.
* `:linker [linker]`  Set/print linker. Supported: `system`, `lld`
* `:timing`           Toggle printing of how long evaluations take
* `:time_passes`      Toggle printing of rustc pass times (requires nightly)
* `:internal_debug`   Toggle internal code debugging output
* `:preserve_vars_on_panic [0|1]`  Try to keep vars on panic

And here are the supported Evcxr commands:

* `:explain`          Print the explanation of last error
* `:clear`            Clear all state, keeping compilation cache
* `:last_compile_dir` Print the directory in which we last compiled
* `:last_error_json`  Print the last compilation error as JSON (for debugging)
* `:dep`              Add an external dependency. e.g. `:dep regex = "1.0"`
* `:help`             View the help message
