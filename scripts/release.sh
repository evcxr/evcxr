#!/bin/bash
set -e
cargo build
cargo test --all
if ! git diff-index --quiet HEAD --; then
  echo "Please commit all changes first" >&2
  exit 1
fi
cd evcxr
cargo publish
cd ../evcxr_repl
cargo publish
cd ../evcxr_jupyter
cargo publish
git push
