# EvCxR Jupyter Kernel

[![Latest Version](https://img.shields.io/crates/v/evcxr_jupyter.svg)](https://crates.io/crates/evcxr_jupyter)

A [Jupyter](https://jupyter.org/) Kernel for the Rust programming language.

## Installation

If you don't already have Rust installed, [follow these
instructions](https://www.rust-lang.org/tools/install).

Make sure you've got a recent version of rust installed. Evcxr's dependencies
often make use of new Rust features shortly after they're stabilized, so it's
not uncommon that the latest release of Evcxr will end up requiring the latest
version of rustc.

### Linux (Debian/Ubuntu)

```sh
rustup component add rust-src
sudo apt install jupyter-notebook cmake build-essential
cargo install evcxr_jupyter
evcxr_jupyter --install
```

Alternative instructions if you want to use libzmq from your system:
```sh
sudo apt install libzmq3-dev jupyter-notebook
cargo install evcxr_jupyter --no-default-features
evcxr_jupyter --install
```

Once installed, you should be able to start Juypter notebook with:

```sh
jupyter-notebook
```

Once it starts, it should open a new tab in your browser, or at least print a
link for you to open. From this tab you can select File -> New Notebook -> Rust.

### Mac OS X

* Install jupyter or jupyterlab (eg. via anaconda)
* Install cmake. [These
  instructions](https://stackoverflow.com/questions/30668601/installing-cmake-command-line-tools-on-a-mac)
  might help.

```sh
cargo install evcxr_jupyter
evcxr_jupyter --install
```

Alternative instructions if you want to use libzmq from your system. Note, if
following these instructions, then you shouldn't need to install cmake.

```sh
brew install zeromq pkg-config
cargo install evcxr_jupyter --no-default-features
evcxr_jupyter --install
```

### Windows

Note that Evcxr on Windows appears to be substantially slower than on other
platforms. We're not yet sure why.

* Install jupyter or jupyterlab (eg. via anaconda)
* Install [CMake](https://cmake.org/download/)
```sh
cargo install evcxr_jupyter
evcxr_jupyter --install
```

If you'd like to install ZMQ yourself, rather than having cargo install build it
for you, then [These
instructions](https://github.com/google/evcxr/issues/53#issuecomment-530050850)
might help.

## Usage information

Evcxr is both a REPL and a Jupyter kernel. See [Evcxr common
usage](https://github.com/google/evcxr/blob/main/COMMON.md) for information that is common
to both.

## Custom output

The last expression in a cell gets printed. By default, we'll use the debug
formatter to emit plain text. If you'd like, you can provide a function to show
your type (or someone else's type) as HTML (or an image). To do this, the type
needs to implement a method called ```evcxr_display``` which should then print
one or more mime-typed blocks to stdout. Each block starts with a line
containing BEGIN\_EVCXR\_OUTPUT followed by the mime type, then a newline, the
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
cargo install --force --git https://github.com/google/evcxr.git evcxr_jupyter
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

* Dr. Shahin Rostami has written a book [Data Analysis with Rust
  Notebooks](https://datacrayon.com/shop/product/data-analysis-with-rust-notebooks/). He's also put
  up a great [getting started video](https://www.youtube.com/watch?v=0UEMn3yUoLo).

## Limitations

* Don't ask Jupyter to "interrupt kernel", it won't work. Rust threads can't be
  interrupted.

## Uninstall

```sh
evcxr_jupyter --uninstall
cargo uninstall evcxr_jupyter
```
