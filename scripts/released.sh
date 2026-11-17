#!/usr/bin/env bash
# Writes released.lock from the parts and Blocks in a registry source. Run it when tagging a
# release, then commit the file. Until the first tag the committed file is empty, because no
# version has been served. The hash is the folder hash parts.lock records.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
usage: scripts/released.sh [<src>]

Write <src>/released.lock, one line `<id> <version> sha256:<hash>` per part and
Block in <src>, sorted by id. <src> defaults to this repo. Run it when tagging
a release, and commit the file with the tag. Until the first tag the committed
file is empty, because no version has been served.
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
