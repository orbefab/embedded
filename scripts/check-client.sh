#!/usr/bin/env bash
# The newest published orbe reads every part and Block in this tree, or refuses
# the item because its format is newer than that binary reads. Any other
# refusal fails. A key that release refuses has to raise the part format.
# Then that release runs `orbe add` for every composite part and Block against
# the registry scripts/registry.sh --with-tree writes from the release tags and
# this tree, and has to install a version whose parts it all reads, or say the
# item needs a newer orbe. Last, orbe 0.2.0 installs a part it reads from a
# registry that also lists one it cannot.
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

cargo build -p orbe --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
scripts/registry.sh --with-tree "$target/debug/orbe" "$work/registry"
python3 scripts/check_client.py --released-registry "$work/orbe" "$work/registry"

# orbe 0.2.0 installs a part it can read from a registry that also lists one it cannot.
old=$work/v0.2.0
mkdir -p "$old"
gh release download v0.2.0 -R orbefab/orbe -D "$old" --pattern 'orbe-*-x86_64-unknown-linux-gnu.tar.gz'
old_archive=$(find "$old" -maxdepth 1 -name 'orbe-*-x86_64-unknown-linux-gnu.tar.gz' -print -quit)
if [[ -z "$old_archive" ]]; then
  echo "scripts/check-client.sh: v0.2.0 has no Linux binary" >&2
  exit 1
fi
tar -xzf "$old_archive" -C "$old"
if [[ ! -x "$old/orbe" ]]; then
  echo "scripts/check-client.sh: the v0.2.0 archive has no orbe binary" >&2
  exit 1
fi
python3 scripts/check_client.py --released-add "$old/orbe"
