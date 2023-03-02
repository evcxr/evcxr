# How to Contribute

We'd love to accept your patches and contributions to this project. All
contributions must be dual licensed Apache2/MIT unless otherwise stated. If you
copy someone else's code, please make sure they are credited and any license
requirements are met.

## Git workflow

Feel free to do your pull requests however you like. You're welcome to either
amend your commits, or add fixup commits. If you add fixup commits, then we'll
squash and rebase your PR when we merge it. If you've got more than one commit
and you'd like to keep them separate when they're merged, then it's probably
best to squash any fixups into the relevant original commit.

## Cargo fmt

When you send a pull request a github action will make sure it builds and passes
tests. It will also check that the code is formatted according to rustfmt. To
save extra cycles, it's recommended to run `cargo fmt` before you commit your
changes.

## Community Guidelines

This project aims to follow the same [code of
conduct](https://www.rust-lang.org/policies/code-of-conduct) as Rust. If there's
a problem, please contact [David Lattimore](https://github.com/davidlattimore).

## Testing

When running tests, it may be useful to run them as follows:

```sh
EVCXR_TMPDIR=$HOME/tmp/e1 cargo test -- --test-threads 1
```

Setting the tmpdir means the generated code doesn't get cleaned up and you can
view it when things go wrong.

Using only a single test thread is currently necessary. Using multiple
evaluation contexts simultaneously currently doesn't work. I haven't
investigated this since in practice it doesn't come up besides in tests.

## Questions?

If you're not sure, feel free to submit a draft PR, file an issue or whatever
works best.
