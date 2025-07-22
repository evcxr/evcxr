# Version 0.21.1
* Fix compilation on Windows.
* Reinstate CI for Windows and Mac.

# Version 0.21.0
* Jupyter kernel: Wait for stdout/stderr to empty before indicating idle. #414 - Thanks thomasjm.
* Update internal rust-analyzer
* MSRV is now 1.88

# Version 0.20.0
* jupyter: Revert a change to when iopub idle is sent. Fixes #400
* Update internal rust-analyzer
* MSRV is now 1.86

# Version 0.19.0
* Update internal rust-analyzer
* jupyter: Ensure iopub idle isn't sent before stdout output
* Add support for shell commands (thanks wiseaidev and drendog)
* User code is now compiled with rust 2024 edition
* MSRV is now 1.85

# Version 0.18.0
* Update dependencies. Fixes compilation without `--locked` due to non-semver breaking change in
  `futures-task`.
* Update rust-analyzer
* Improvements to .toml parsing (baiguoname)
* Fix error display when variable type cannot be inferred and there's a trailing expression.
* MSRV is now 1.80 (due to dependency changes)

# Version 0.17.0
* Reverted to static linking by default as we had prior to 0.16.0. You can still get dynamic linking
  by setting `:allow_static_linking 0` which is recommended if it works for you. Forcing dynamic
  linking was breaking in hard-to-debug ways for several people on both Mac and Linux.
* Fixes for async-await support.
* Added commands to set runtime environment variables (`:env`) and build environment variables
  (`:build_env`).
* An `evcxr.toml` in your startup directory can now be used to override your target-dir. Thanks
  baiguoname.
* Added support for selecting rustc's codegen backend. You can now use the cranelift backend by
  doing `:toolchain nightly` then `:codegen_backend cranelift`.
* Updated rust-analyzer
* Minimum rust version is now 1.74 due to changes in rust-analyzer

# Version 0.16.0
* Now compiles dependencies as dylibs. This means that mutable static variables in dependencies are
  now preserved between executions. If you hit problems with this, please file a bug report. You can
  restore the old behaviour with `:allow_static_linking 1`.
* New built-in caching mechanism. Enable a 500MiB cache by adding `:cache 500` to your
  `~/.config/evcxr/init.evcxr`.
* Use of sccache is now deprecated, since it doesn't work with dylibs. Switching to the new caching
  mechanism is recommended.
* Update to latest rust-analyzer
* New command to show current dependencies: `:show_deps`. Thanks momori256.
* Fixed some issues with the jupyter kernel not shutting down cleanly.
* Fixed Tokio runtime being poisoned after panic. Thanks martinitus.
* Added a `:doc` command to show documentation for something. Thanks baiguoname.
* Improvements to an error message. Thanks baiguoname.
* Improved docs for how to determine config path. Thanks anandijain.
* Now requires rust version 1.70 or later

# Version 0.15.1
* Fix miscompilation when there's a trailing comment after an expression.
* Fix out-of-order printing in evcxr_jupyter

# Version 0.15.0
* `:dep` now prints "Compiling" messages emitted by Cargo to show progress. Thanks d86leader and
  adiSuper94.
* `:type` (or `:t`) command added to get the type of a single variable. Thanks d86leader.
* `:types` command added to enable the display of evaluated expression types.
* Attributes (e.g. `#[derive(Debug)]`) now cause REPL to wait for an additional line. Thanks
  Marcono1234.
* Fixed `:toolchain` command that was broken in a previous release.
* Suggest offline mode when adding a dependency fails. Thanks Marcono1234.
* Added links to some new resources for the Jupyter kernel. Thanks wiseaidev.
* Escape paths - especially important for use on Windows. Thanks Marcono1234.
* Fix for a file locking issue on Windows.
* Documentation fixes. Thanks Marcono1234.
* init.evcxr is now executed all at once rather than a line at a time.
* Try to use cargo/rustc paths from build time if cargo/rustc aren't on the path at runtime.
* Minimum supported rust version is now 1.67.0.
* Arguments after `--` are now available via `std::env::args` in the REPL.
* CLA no longer required for contributions.
* Repository moved out of Google org into its own "evcxr" org.
* License changed to dual Apache2/MIT (Previously just Apache2).

# Version 0.14.2
* Fixed jupyter kernel running from vscode. Thanks TethysSvensson for bisecting
  the cause.
* Updated rust-analyzer

# Version 0.14.1
* Fixed thread starvation in Jupyter integration on systems with few CPUs.
* Support interrupting execution in Jupyter notebook. Process gets terminated,
  so variables are lost, but other state is preserved.
* Support interrupting execution in REPL by pressing ctrl-c.
* Updated rust-analyzer.

# Version 0.14.0
* `:dep` lines can now be commented out without breaking subsequent `:dep`
  lines. Thanks JohnScience!
* Reduced interleaving of stdout and stderr.
* Errors now render using ariadne, giving a much nicer presentation. Thanks
  HKalbasi!
* Small speedup in execution time by bypassing rustup.
* `preserve_vars_on_panic` is now true by default (as it used to be). It now
  works much better than before, with all variables being preserved if a panic
  occurs.
* By using rustyline's new external printer API, stderr output that hasn't been
  displayed before we return to the prompt no longer messes up the prompt. In
  fact, you can now spawn a thread that writes to stderr in the background and
  the output now appears above your prompt.
* Jupyter kernel now uses a native Rust library for ZMQ, so no longer requires
  cmake to build.
* The last bit of code that tried to use rustc error messages to determine
  variable types has been deleted. Now if rust-analyzer can't determine the type
  of a variable, we ask the user to add an explicit type annotation.
* Update rust-analyzer
* Building evcxr now requires rust 1.63 or higher.
* Added a release workflow so that binaries for the latest Linux (built on
  Ubuntu), Windows and Mac are available for download on the releases page.

# Version 0.13.0
* Now uses Rust edition 2021.
* MSRV is now 1.59.
* Changed completion type in REPL to `list`. See evcxr_repl/README.md if you'd
  like the old behavior.
* Update to latest rust-analyzer - const generics now work reasonably well!
* Fix escaping of ampersands in HTML output - Thanks Tim McNamara.
* Use mold for linking if of path - Thanks Will Eaton.
* Fix inline errors showing up for await calls in Jupyter notebook.

# Version 0.12.0
* Fix compilation due to a non-semver breaking change in an upstream crate.
* Update to latest rust-analyzer.
* Minimum supported rust version is now 1.55 due to changes in rust-analyzer.

# Version 0.11.0
* Update rust-analyzer - fixes a compilation failure.
* Support for crate-level attributes - e.g. `#![feature(...)]`
* Minimum supported rust version is now 1.53 due to changes in rust-analyzer.

# Version 0.10.0
* Use mimalloc. This reduces startup time, at least one Mac. Thanks thomcc.
* Initialize CommandContext in the background. Reduces startup time. Thanks thomcc.
* Updated rustyline. Thanks thomcc
* Use rust-analyzer for inferring types for let destructurings.
* Update rust-analyzer. Fixes evcxr on nightly, beta (and next stable release).
* Minimum supported rust version now 1.52 (required for latest rust-analyzer).

# Version 0.9.0
* Fix relative paths in deps. Thanks Max Göttlicher!
* Use explicit types for variables when supplied. This is currently required for variables that use
  const generics.
* Minimum rust version is now 1.51 (since upstream crates are already using const generics).
* Don't misuse JUPYTER_CONFIG_DIR.
* Make `a = 10` (no semicolon or let) report an error.
* Updated rust-analyzer

# Version 0.8.1
* Fixed bug that was affecting HTML export from Jupyter. #153
* Updated rust-analyzer.

# Version 0.8.0
* Jupyter kernel now shows errors and warnings inline as you type by running cargo check on the
  backend. Running `evcxr\_jupyter --install` before starting jupyter notebook is recommended. It
  will auto-update when the evcxr jupyter kernel starts, however that update may not effect the
  current session.
* Fixed `:clear` command.
* Work around imprecise timestamps on Macs that use a HPFS filesystem
* Added `:toolchain` command to allow specifying rust compiler toolchains (e.g. "nightly").
* Added `:offline` to turn on offline mode when running Cargo.
* Various improvements to error reporting.

# Version 0.7.0
* Fixed a number of bugs in tab completion.
* Documentation improvements. Thanks Ibraheem!
* Fixed a panic when doing tab completion after loading a crate with a
  hyphenated name.
* Changed internal parsing logic to use rust-analyzer instead of syn (reduced
  binary size from 24MB to 20MB).
* Improved semantics for use statements. e.g. `use a::{b, c};` followed by `use
  a::{b, c, d}` now won't give you errors.
* API changes in the evcxr crate (used by the repl and jupyter kernel).

# Version 0.6.0
* Support for rustc 1.48.
* Tab completion based on rust-analyzer (in both Jupyter kernel and REPL).
* Minimum rust version is now 1.46
* Now requires rust-src component
  * rustup component add rust-src
* Jupyter kernel now supports prompting for input.
* REPL now supports the quit command. Thanks komi

# Version 0.5.3
* Fix for a crash in the REPL when certain multiline error were encountered.
* REPL now supports `--edit-mode vi` thanks to aminroosta
* REPL now has history-based tab completion thanks to aminroosta

# Version 0.5.2
* Works on FreeBSD. Thanks dmilith!
* Multiline errors look much better in the REPL thanks to aminroosta!
* Improved cursor navigation in REPL thanks to Ma27!

# Version 0.5.1
* Fixed colored text outputs on Windows. Thanks Mirko and Dmitry!

# Version 0.5.0
* Supports async / await using Tokio as the executor.
* Question mark operator can now be used (errors are printed to stderr).
* Now requires rustc >= 1.40.0
* REPL now support multiline input (thanks Thom Chiovoloni!)
* Don't use lld on MacOS, it's broken (thanks Thom Chiovoloni for figuring this
  out)

# Version 0.4.7
* Fixed segfault if executing code without variables followed by code with
  variables.
* Automatically use lld if it's installed. Use :linker command to override.
* Handle variables that contain type inference placeholders `_`.
* Added support for overriding the config dir with the environment variable
  EVCXR_CONFIG_DIR (thanks Thom Chiovoloni).
* Run prelude.rs after init.evcxr if one exists (thanks Thom Chiovoloni).
* Allow output format to be specified. e.g. :fmt {:#?} (thanks κeen)

# Version 0.4.6
* Fixes to work with rust 1.41
* Improved handling of cargo workspaces.
* Now works if the user has overridden Cargo's default target directory (thanks
  Aloxaf).
* Fixed prompt color (thanks Dmitry Murzin).
* Added a flag to disable use of readline (thanks Dmitry Murzin).
* Fix for binder (thanks Luiz Irber)

# Version 0.4.5
* Escape reserved words "async" and "try" when encountered in types.
* Use vendored ZMQ library by default.

# Version 0.4.4
* Now support [sccache](https://github.com/mozilla/sccache).
* These release notes previously said that this release added support for mixing
  commands like :dep with code. That feature was actually added a long time ago
  by David Bernard. Thanks David, and sorry for the confusion.

# Version 0.4.3
* No longer preserves variables on panic by default.
  * Turns out this was significantly slowing down compilation.
  * You can get back the old behavior with `:preserve_vars_on_panic 1`
  * Put that in your ~/.config/evcxr/init.evcxr or equivalent to always have it.
* Optimization is now back on by default. With the above change, there's now not
  really any noticeable difference in eval times for small amounts of code.

# Version 0.4.2
* Fixed runtime error on windows due to something not liking the dll having been
  renamed.
* Added option :preserve_vars_on_panic, which still defaults to on, but which if
  turned off will speed up some compilations at the expense of all variables
  being lost if a panic occurs.

# Version 0.4.1
* Revert change to not preserve copy variables on panic as it broke mutation of
  copy variables. Will reenable in future once it's properly fixed.

# Version 0.4.0
* Optimization is now off by default, since many people using a REPL or Jupyter
  kernel are experimenting and faster compile times are more important than
  faster runtimes.
    * If you want it always on, see README.md for how to do that.
* New execution model.
  * A single crate is now reused for all compilation. This is a bit faster than
    the old model where each execution was a separate crate that had a
    dependency on the previous crates.
  * Defined items no longer need to be pub.
* Optimization level can now be set (as opposed to just toggled).
* Reads commands (one per line) from a startup file.
  * e.g. ~/.config/evcxr/init.evcxr (on Linux)
* Now uses Rust 2018 edition.
* Don't preserve variables that are Copy on panic.
  * Results in a small speedup in some evaluation times.
  * If you really want this, you can opt back in via :preserve_copy_types.

# Version 0.3.5
* Fix for another upcoming cargo change (due in 1.37).

# Version 0.3.4
* Fix with upcoming beta release (1.36) where Cargo started intercepting and
  wrapping JSON errors from the compiler.
* Give proper error message if a closure or an impl trait is stored into a
  variable.
* Recover from compilation failure in a case where we previously got out-of-sync
  with what variables should exist.

# Version 0.3.3
* Windows and Mac support! Big thanks to Daniel Griffen for the final fixes and
  David Bernard for Travis setup.

# Version 0.3.1
* Fixed handling of crates with "-" in their name.
* Support relative crate paths.
* Don't error if the same extern crate is given multiple times with slightly
  different formatting.

# Version 0.3.0
* Fix optimization (wasn't actually working before for some reason).
* Give better error message if rustc suggests a private type for a variable.
* Allow variables to be given explicit types.
* A couple of fixes for Windows (probably not enough for it to actually work
  though, but it's a start).
* Support for running in Binder.

# Version 0.2.0

* :dep no longer automatically adds extern crate. extern crate still
  automatically adds a dependency, but only if there isn't already a library
  with the specified name.
* Including ":help" command now works properly in the Jupyter kernel.
* Various fixes related to running on MacOS.
* Numerous other improvements to the Jupyter kernel, in particular making it
  work with Jupyter Lab.
