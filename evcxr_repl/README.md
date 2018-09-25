# Evcxr REPL

A REPL (Read-Eval-Print loop) for Rust.

## Installation and usage

Only tested on Linux so far. I don't think it should take too much to get going
on other platforms, but I don't have those other platforms. Help very welcome.

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
  * e.g. ```extern crate regex;``` will load the latest version of the regex crate.
  * This can take a while, especially for large crates with many dependencies.
* Expressions will be debug printed.
* For the most part compilation errors are reported in a reasonably intuitive inline way.

## Limitations

* Only tested on Linux. I don't have Windows or Mac. I've tried to be as
  platform agnostic as possible, but there's almost certain to be issues. Drop
  me an email if you'd like to help get it working on another platform.
* Storing references into variables that persist between compilations is not permitted.
* Functions, structs etc must be explicitly made pub, otherwise you'll not be
  able to reference them later on. I'd like to make them pub automatically, but
  it's a bit of work, since error spans will need fixing to compensate for the
  added text. Also, it's not really possibly until the spans used by syn are
  stabalized.
* Not yet any way to import macros from external crates.
* Since each line is compiled as a separate crate, impls generally need to go on
  the same line as the type they're for.

## Similar projects

* [rusti](https://github.com/murath/rusti). From a quick look, it appears to
  require a nightly compiler from 2016 and doesn't appear to persist variable
  values. So I suspect the way that it works is pretty different.
