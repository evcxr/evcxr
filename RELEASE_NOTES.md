# Version 0.4.0
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
