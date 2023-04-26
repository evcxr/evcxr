# Evcxr Jupyter Kernel

[![Binder](https://mybinder.org/badge.svg)](https://mybinder.org/v2/gh/evcxr/evcxr/main?filepath=evcxr_jupyter%2Fsamples%2Fevcxr_jupyter_tour.ipynb)

[![Latest Version](https://img.shields.io/crates/v/evcxr_jupyter.svg)](https://crates.io/crates/evcxr_jupyter)

A [Jupyter](https://jupyter.org/) Kernel for the Rust programming language.

## Installation

If you don't already have Rust installed, [follow these
instructions](https://www.rust-lang.org/tools/install).

You can either download a pre-built binary from the
[Releases](https://github.com/evcxr/evcxr/releases) page, extract it from the
archive and put it somewhere on your path, or build from source by running:
```sh
cargo install evcxr_jupyter
```

Whether using a prebuilt binary or one you built yourself, you'll need to run
the following command in order to register the kernel with Jupyter.

```sh
evcxr_jupyter --install
```

By default, `evcxr_jupyter --install` will install the kernel into a user local
directory, e.g., `$HOME/.local/share/jupyter/kernels`.

If your operating system is an older version, or has a different libc than what
the pre-built binaries were compiled with, then you'll need to build from source
using the command above.

To actually use evcxr_jupyter, you'll need Jupyter notbook to be installed.
* Debian or Ubuntu Linux: `sudo apt install jupyter-notebook`
* Mac: You might be able to `brew install jupyter`
* Windows, or if the above options don't work for you, see
  https://jupyter.org/install

You'll also need the source for the Rust standard library installed. If you
already use rust-analyzer, you'll likely have this installed. To install this
using rustup, run:
```sh
rustup component add rust-src
```

## Running

To start Jupyter Notebook, run:

```sh
jupyter notebook
```

Once started, it should open a page in your web browser. Look for the "New" menu
on the right and from it, select "Rust".

## Usage information

Evcxr is both a REPL and a Jupyter kernel. See [Evcxr common
usage](https://github.com/evcxr/evcxr/blob/main/COMMON.md) for information that is common
to both.

## Custom output

The last expression in a cell gets printed. By default, we'll use the debug
formatter to emit plain text. If you'd like, you can provide a function to show
your type (or someone else's type) as HTML (or an image). To do this, the type
needs to implement a method called ```evcxr_display``` which should then print
one or more mime-typed blocks to stdout. Each block starts with a line
containing EVCXR\_BEGIN\_CONTENT followed by the mime type, then a newline, the
content then ends with a line containing EVCXR\_END\_CONTENT.

For example, the following shows how you might provide a custom display function for a
type Matrix. You can copy this code into a Jupyter notebook cell to try it out.

```rust
use std::fmt::Debug;
pub struct Matrix<T> {pub values: Vec<T>, pub row_size: usize}
impl<T: Debug> Matrix<T> {
    pub fn evcxr_display(&self) {
        let mut html = String::new();
        html.push_str("<table>");
        for r in 0..(self.values.len() / self.row_size) {
            html.push_str("<tr>");
            for c in 0..self.row_size {
                html.push_str("<td>");
                html.push_str(&format!("{:?}", self.values[r * self.row_size + c]));
                html.push_str("</td>");
            }
            html.push_str("</tr>");
        }
        html.push_str("</table>");
        println!("EVCXR_BEGIN_CONTENT text/html\n{}\nEVCXR_END_CONTENT", html);
    }
}
let m = Matrix {values: vec![1,2,3,4,5,6,7,8,9], row_size: 3};
m
```

It's probably a good idea to either print the whole block at once, or to lock
stdout then print the block. This should ensure that nothing else prints to
stdout at the same time (at least no other Rust code).

If the content is binary (e.g. mime type "image/png") then it should be base64
encoded.

## Prompting for input

```rust
:dep evcxr_input
let name = evcxr_input::get_string("Name?");
let password = evcxr_input::get_password("Password?");
```

## Installing from git head

If there's a bugfix in git that you'd like to try out, you can install directly
from git with the command:

```sh
cargo install --force --git https://github.com/evcxr/evcxr.git evcxr_jupyter
```

## 3rd party integrations

There are several Rust crates that provide Evcxr integration:

* [Petgraph](https://crates.io/crates/petgraph-evcxr)
  * Graphs (the kind with nodes and edges)
* [Plotly](https://igiagkiozis.github.io/plotly/content/fundamentals/jupyter_support.html)
  * Lots of different kinds of charts
* [Plotters](https://crates.io/crates/plotchart#trying-with-jupyter-evcxr-kernel-interactively)
  * Charts
* [Showata](https://crates.io/crates/showata)
  * Displays images, vectors, matrices (nalgebra and ndarray)

## 3rd party resources

* Eng. Mahmoud Harmouch has written a series of articles and developed a list of Jupyter notebooks
  equipped with all the tools needed for various data analysis tasks that are documented in [this
  repository](https://github.com/wiseaidev/rust-data-analysis).

* Dr. Shahin Rostami has written a book [Data Analysis with Rust
  Notebooks](https://datacrayon.com/shop/product/data-analysis-with-rust-notebooks/). It uses
  library versions that are a bit out of date now, but the examples still work. He's also put up a
  great [getting started video](https://www.youtube.com/watch?v=0UEMn3yUoLo).

## Limitations

* Don't ask Jupyter to "interrupt kernel", it won't work. Rust threads can't be
  interrupted.

## Uninstall

```sh
evcxr_jupyter --uninstall
cargo uninstall evcxr_jupyter
```
