#!/usr/bin/env bash
# Writes released.lock from the parts and Blocks in a registry source. Run it when tagging a
# release, then commit the file. Until the first tag the committed file is empty, because no
# version has been served. The hash is the SHA-256 of the record the registry build writes.
# A line for an item that is no longer in the tree stays. A version never moves backwards.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
usage: scripts/released.sh [<src>]

Write <src>/released.lock, one line `<id> <version> sha256:<hash>` per part and
Block a release served, sorted by id. The hash is the SHA-256 of the record
`orbe registry build` writes for that id and version. A line whose folder is
gone stays. A line is added or advanced, and a version never moves backwards.
<src> defaults to this repo. Run it when tagging a release, and commit the file
with the tag. Until the first tag the committed file is empty, because no
version has been served.
EOF
}

if [[ ${1:-} == -h || ${1:-} == --help ]]; then
  usage
  exit 0
fi
if [[ $# -gt 1 ]]; then
  usage >&2
  exit 2
fi
src=${1:-$PWD}
if [[ ! -d $src ]]; then
  echo "scripts/released.sh: $src is not a directory" >&2
  exit 2
fi
src=$(cd "$src" && pwd)
cargo run -p orbe-registry --bin released --locked --quiet -- "$src"
