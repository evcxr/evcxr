# Evcxr Runtime

[![Latest Version](https://img.shields.io/crates/v/evcxr_runtime.svg)](https://crates.io/crates/evcxr_runtime)

Provides functionality that may be of use by code running inside Evcxr. In
particular inside the Evcxr Jupyter kernel.

At the moment, all that's provided is functions and traits for emitting
mime-typed data to Evcxr.

```
impl evcxr_runtime::Display for MyType {
    fn evcxr_display(&self) {
        evcxr_runtime::mime_type("text/html")
            .text("<span style=\"color: red\">>Hello world</span>");
    }
}
```
