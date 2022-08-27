#!/bin/bash
set -e
if ! git diff-index --quiet HEAD --; then
  echo "Please commit all changes first" >&2
  exit 1
fi
MIN_RUST_VER=$(grep MSRV .github/workflows/ci.yml | awk '{print $2}')
if [ -z "$MIN_RUST_VER" ]; then
  echo "Failed to determine minimum rust version" >&2
  exit 1
fi
git pull --rebase
cargo fmt --all -- --check
cargo build
cargo +stable test --all
cargo +nightly test --all
cargo +${MIN_RUST_VER}-x86_64-unknown-linux-gnu test --all
git push
