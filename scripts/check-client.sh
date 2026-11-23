#!/usr/bin/env bash
# The newest published orbe reads every part and Block in this tree, or refuses
# the item because its format is newer than that binary reads. Any other
# refusal fails. The published release and the registry share one tree, so a
# key that release refuses has to raise the part format.
set -euo pipefail

cd "$(dirname "$0")/.."

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

gh release download -R orbefab/orbe -D "$work" --pattern 'orbe-*-x86_64-unknown-linux-gnu.tar.gz'
archive=$(find "$work" -maxdepth 1 -name 'orbe-*-x86_64-unknown-linux-gnu.tar.gz' -print -quit)
if [[ -z "$archive" ]]; then
  echo "scripts/check-client.sh: the newest release has no Linux binary" >&2
  exit 1
fi
tar -xzf "$archive" -C "$work"
if [[ ! -x "$work/orbe" ]]; then
  echo "scripts/check-client.sh: the archive has no orbe binary" >&2
  exit 1
fi
python3 scripts/check_client.py "$work/orbe"
