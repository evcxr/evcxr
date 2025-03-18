# Evcxr common usage information

This page contains usage information that is common to both the [Evcxr REPL](evcxr_repl/README.md)
and the [Evcxr Jupyter kernel](evcxr_jupyter/README.md).

## Usage notes

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

If you'd like to use a crate with a different name than what it's know as on crates.io, you can use
`:dep any_name = { package = "crates_io_name" }`. For example, if you wanted to load the crates.io
package `unicode-xid`, but refer to it locally as `unicode_xid`, you could do that as follow.

```rust
:dep unicode_xid = { package = "unicode-xid", version = "*" }
use unicode_xid;
```

You can use the local work-in-progress crate like this:

```rust
>> :dep my_crate = { path = "." }
>> use my_crate::*;
```

Alternatively you can use a shorter form:

```rust
>> :dep .
>> :dep ../another_crate
>> :dep /path/to/crate
```

It will even automatically update when you save your files!

There are many other options that can be specified. See Cargo's [official dependency
documentation](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html) for details.

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

**Support for async-await**

If you use `await` in your code, evcxr will automatically build and start a Tokio runtime. You can
test this out with a trival example as follows:

```
>> async fn foo() -> u32 { 42 }
>> foo().await
   Compiling libc v0.2.150
   Compiling num_cpus v1.16.0
   Compiling tokio v1.34.0
42
```

If you'd like to use more non-default features of tokio, then you need to add the dependency on
tokio before you first try to use the await keywork. For example:

```
:dep tokio = {version = "1.34.0", features = ["full"]}
```

You can then write code that uses await and can optionally use the `?` operator as well.

The following code will attempt to connect to localhost on port 1234.

```
let mut stream = tokio::net::TcpStream::connect("127.0.0.1:1234").await?;
```

Unless you happen to have a program listening on this port, this should report the error:

```
Connection refused (os error 111)
```

If you're on Linux, you can use netcat (you may need to install it) to listen on an arbitrary port.
For example:

```sh
nc -t -l 1234
```

Leave that running in one shell, then back in evcxr, run the following:

```rust
use tokio::io::AsyncWriteExt;
stream.write(b"Hello, world!\n").await?;
```

You should hopefully see the "Hello, world!" message appear in the shell where netcat (nc) is
running. You can stop nc by pressing control-c.

## Limitations

* There is currently no way to import macros from external crates.

## Documentation

### Startup

You can create an `init.evcxr` file in the `evcxr` config directory. The location of this directory varies depending on your operating system:

| Linux             | OSX                                      | Windows                                |
|-------------------|------------------------------------------|----------------------------------------|
|` ~/.config/evcxr` | `/Users/Alice/Library/Preferences/evcxr` | `C:\Users\Alice\AppData\Roaming\evcxr` |

You can check the location of the config directory by running the following in the REPL:
```rust 
:dep dirs
dirs::config_dir().unwrap().join("evcxr").join("init.evcxr")
```

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

You can optionally cache compilation outputs. To do so, add `:cache {size in MB}` to your
`init.evcxr`. e.g. to have a 500 MB cache, add the following:

```
:cache 500
```

To disable the cache, use `:cache 0`. Running with the cache disabled doesn't clear the cache. To
clear the cache, run `:clear_cache`.

### Variable Persistence

The `:vars` command will list all the variables defined in the current context:
```rust
>> let x = 0;
>> let y = 1;
>> :vars
y: i32
x: i32
```

### References

Variables that persist cannot reference other variables. For example, you can't do this:

```rust
let all_values = vec![10, 20, 30, 40, 50];
let some_values = &all_values[2..3];
```

There are a few ways to mitigate this limitation. Firstly, if you don't need `some_values` to
persist, you can limit its scope:

```rust
let all_values = vec![10, 20, 30, 40, 50];
{
    let some_values = &all_values[2..3];
    // Use some_value here
}
```

If you really need `some_values` to persist, you can make `all_values` into a static reference by
leaking it:

```rust
let all_values: &'static Vec<i32> = Box::leak(Box::new(vec![10, 20, 30, 40, 50]));
let some_values = &all_values[2..3];
```

Note that we need to give `all_values` a type here because otherwise the type ends up being a
mutable reference, which would result in us still having borrow checker problems.

### Linker

Installing the [`lld`](https://lld.llvm.org/) linker it is recommended as it is generally faster than the default system linker. On Debian-based systems you might be able to install it with:
```sh
$ sudo apt install lld
```
`lld` will be used automatically if detected on all systems with the exception of Mac OS. You can check which linker is being used by running `:linker`.

### Using cranelift backend

If you'd like to try using cranelift rather than llvm to do codegen, you can do so using recent rust
nightly releases.

First update your nightly toolchain with `rustup update nightly` or, if you don't already have it
installed, `rustup install nightly`.

Install the cranelift preview component:

```sh
rustup component add rustc-codegen-cranelift-preview --toolchain nightly
```

Then use the following commands from evcxr or in your init.evcxr.

```
:toolchain nightly
:codegen_backend cranelift
```

### Commands

Here is a complete list of the configuration options you can set to customize your Evcxr experience:

* `:efmt [format]`    Set the formatter for errors returned by `?`
* `:fmt [format]`     Set output formatter (default: `{:?}`)
* `:internal_debug`   Toggle internal code debugging output
* `:linker [linker]`  Set/print linker. Supported: `system`, `lld`, `mold`
* `:offline [0|1]`    Set offline mode when invoking cargo
* `:opt [level]`      Toggle/set optimization level
* `:preserve_vars_on_panic [0|1]`  Try to keep vars on panic
* `:sccache [0|1]`    Set whether to use sccache
* `:time_passes`      Toggle printing of rustc pass times (requires nightly)
* `:timing`           Toggle printing of how long evaluations take
* `:toolchain`        Set which toolchain to use (e.g. nightly)
* `:types`            Toggle printing of the type of the output

And here are the supported Evcxr commands:

* `:clear`            Clear all state, keeping compilation cache
* `:dep`              Add an external dependency. e.g. `:dep regex = "1.0"`
* `:explain`          Print the explanation of last error
* `:help`             View the help message
* `:last_compile_dir` Print the directory in which we last compiled
* `:last_error_json`  Print the last compilation error as JSON (for debugging)
* `:load_config`      Reloads startup configuration files. Accepts optional flag `--quiet` to suppress logging.
* `:quit`             Quit evaluation and exit
* `:type` | `:t`      Show variable type
* `:vars`             List bound variables and their types
* `:version`          Print Evcxr version
