# Evcxr

[![Binder](https://mybinder.org/badge.svg)](https://mybinder.org/v2/gh/google/evcxr/main?filepath=evcxr_jupyter%2Fsamples%2Fevcxr_jupyter_tour.ipynb)

An evaluation context for Rust.

This project consists of several related crates.

* [evcxr\_jupyter](evcxr_jupyter/README.md) - A Jupyter Kernel

* [evcxr\_repl](evcxr_repl/README.md) - A Rust REPL

* [evcxr](evcxr/README.md) - Common library shared by the above crates, may be
  useful for other purposes.

* [evcxr\_runtime](evcxr_runtime/README.md) - Functions and traits for
  interacting with Evcxr from libraries that users may use from Evcxr.
  
If you think you'd like a REPL, I'd definitely recommend checking out the
Jupyter kernel. It's pretty much a REPL experience, but in a web browser.

To see what it can do, it's probably best to start with a [tour of the Jupyter
kernel](evcxr_jupyter/samples/evcxr_jupyter_tour.ipynb). Github should allow you
to preview this, or you can load it from Jupyter Notebook and run it yourself.

## Disclaimer

This is not an officially supported Google product. It's released by Google only
because the (original) author happens to work there.
