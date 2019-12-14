#!/bin/bash
set -e
if [ $# -ne 1 ]; then
  echo "Usage: $0 {version}" >&2
  exit 1
fi
if ! git diff-index --quiet HEAD --; then
  echo "Please commit all changes first" >&2
  exit 1
fi
VERSION="$1"
if ! grep "# Version $VERSION" RELEASE_NOTES.md >/dev/null; then
  echo "Please add release notes first" >&2
  exit 1
fi
git pull --rebase
perl -pi -e 's/^version = "[\d\.]+"/version = "'$VERSION'"/;\
    s/^evcxr = \{ version = "=[\d\.]+"/evcxr = \{ version = "='$VERSION'"/' \
  evcxr/Cargo.toml \
  evcxr_repl/Cargo.toml \
  evcxr_jupyter/Cargo.toml
cargo build
cargo test --all
git commit -a -m "Bump vesion to $VERSION"
cd evcxr
cargo publish
# Wait a but before we try to push packages that depend on the version we just
# pushed above, otherwise the push seems to fail. Seems like write followed by
# read gives stale results!
sleep 60
cd ../evcxr_repl
cargo publish
cd ../evcxr_jupyter
cargo publish
git tag "v$VERSION"
git push
