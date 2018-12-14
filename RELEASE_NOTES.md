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
