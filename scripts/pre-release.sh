#!/bin/bash
set -e
VERSION=$(perl -e \
  'while (<>) { if (/^# Version (\d+\.\d+\.\d+) \(unreleased\)/) {print "$1"}}' \
  RELEASE_NOTES.md \
)
if [ -z "$VERSION" ]; then
  echo "RELEASE_NOTES.md doesn't contain an unreleased version" >&2
  exit 1
fi
if ! git diff-index --quiet HEAD --; then
  echo "Please commit all changes first" >&2
  exit 1
fi
MIN_RUST_VER=$(grep ^rust-version evcxr/Cargo.toml | cut -d'"' -f2)
if [ -z "$MIN_RUST_VER" ]; then
  echo "Failed to determine minimum rust version" >&2
  exit 1
fi
echo "Min rust version $MIN_RUST_VER"
echo "Releasing $VERSION"
git pull --rebase
perl -pi -e 's/(^# .*) \(unreleased\)$/$1/' RELEASE_NOTES.md
perl -pi -e 's/^version = "[\d\.]+"/version = "'$VERSION'"/;\
    s/^evcxr = \{ version = "=[\d\.]+"/evcxr = \{ version = "='$VERSION'"/' \
  evcxr/Cargo.toml \
  evcxr_repl/Cargo.toml \
  evcxr_jupyter/Cargo.toml
cargo build
cargo +stable test --all
cargo +nightly test --all
cargo +${MIN_RUST_VER}-x86_64-unknown-linux-gnu test --all
git commit -a -m "Bump vesion to $VERSION"
git rev-parse HEAD >.pre-release.hash
