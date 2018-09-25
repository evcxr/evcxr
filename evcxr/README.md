# Evcxr library

An implementation of eval() for Rust.

The main struct in this crate is ```EvalContext```. You create one, then ask it
to eval bits of code. Any defined functions, variables etc are local to that
context.

I'll not go into too much detail here, since this purpose of this library is
really to provide functionality to evcxr\_jupyter and evcxr\_repl. If you'd like
to try using this crate for something else, drop me an email, or file an issue
on the repository and we can figure out your use case.

## How it works

See [how it works](HOW_IT_WORKS.md)
