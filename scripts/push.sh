#!/bin/bash
set -ev
if ! git diff-index --quiet HEAD --; then
  echo "Please commit all changes first" >&2
  exit 1
fi
MIN_RUST_VER=$(grep ^rust-version evcxr/Cargo.toml | cut -d'"' -f2)
if [ -z "$MIN_RUST_VER" ]; then
  echo "Failed to determine minimum rust version" >&2
  exit 1
fi
fail() {
  echo "$@" >&2
  exit 1
}
cargo +${MIN_RUST_VER} --version >/dev/null 2>&1 \
  || rustup toolchain install $MIN_RUST_VER
git pull --rebase
cargo fmt --all -- --check
cargo acl
if ! git diff-index --quiet HEAD --; then
  echo "Please commit all changes first" >&2
  exit 1
fi
cargo build
cargo clippy
cargo +stable test --all || fail "Tests failed on stable"
cargo +nightly test --all || fail "Tests failed on nightly"
cargo +${MIN_RUST_VER}-x86_64-unknown-linux-gnu test --all \
  || fail "Tests failed on $MIN_RUST_VER"
git push
