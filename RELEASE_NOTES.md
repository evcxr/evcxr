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
  really any noticable difference in eval times for small amounts of code.

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
* Fix with upcomming beta release (1.36) where Cargo started intercepting and
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
* A couple of fixes for Windows (probaly not enough for it to actually work
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
