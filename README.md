# Evcxr

[![Binder](https://mybinder.org/badge_logo.svg)](https://mybinder.org/v2/gh/evcxr/evcxr/main?filepath=evcxr_jupyter%2Fsamples%2Fevcxr_jupyter_tour.ipynb)

An evaluation context for Rust.

This project consists of several related crates.

* [evcxr\_jupyter](evcxr_jupyter/README.md) - A Jupyter Kernel

* [evcxr\_repl](evcxr_repl/README.md) - A Rust REPL

* [evcxr](evcxr/README.md) - Common library shared by the above crates, may be useful for other
  purposes.

* [evcxr\_runtime](evcxr_runtime/README.md) - Functions and traits for interacting with Evcxr from
  libraries that users may use from Evcxr.
  
If you think you'd like a REPL, I'd definitely recommend checking out the Jupyter kernel. It's
pretty much a REPL experience, but in a web browser.

To see what it can do, it's probably best to start with a [tour of the Jupyter
kernel](evcxr_jupyter/samples/evcxr_jupyter_tour.ipynb). Github should allow you to preview this, or
you can load it from Jupyter Notebook and run it yourself.

## License

This software is distributed under the terms of both the MIT license and the Apache License (Version
2.0).

See LICENSE for details.
