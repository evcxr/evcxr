#!/bin/bash
set -e

VERSION=$(grep ^version evcxr/Cargo.toml | cut -d'"' -f2)

if [ -z "$VERSION" ]; then
  echo "Couldn't determine version" >&2
  exit 1
fi

if [ $(git rev-parse HEAD) != $(cat .pre-release.hash) ]; then
  echo "Please run ./scripts/pre-release.sh first" >&2
  exit 1
fi

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
#git tag "v$VERSION"
#git push origin
#git push origin refs/tags/v$VERSION
