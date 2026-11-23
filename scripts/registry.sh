#!/usr/bin/env bash
# Builds the registry the site serves from release tags v0.2.0 and later.
# An older tag's record at id@version stays. The newest tag supplies the index
# name and the schemas. Each index row names the lowest orbe whose part formats
# include that record's format. A format no tag reads names the patch after
# the newest tag.
#
# usage: scripts/registry.sh <orbe> <out>
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $# -ne 2 ]]; then
  echo "usage: scripts/registry.sh <orbe> <out>" >&2
  exit 2
fi
orbe=$1
out=$2
if [[ ! -x $orbe ]]; then
  echo "scripts/registry.sh: $orbe is not an orbe binary" >&2
  exit 1
fi

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "scripts/registry.sh: this is not a git checkout, so there are no release tags to publish" >&2
  exit 1
fi
if ! git rev-parse -q --verify v0.2.0 >/dev/null 2>&1; then
  git fetch --tags --force origin
fi

mapfile -t tags < <(git tag -l 'v*' | while read -r tag; do
  ver=${tag#v}
  [[ $ver =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || continue
  lowest=$(printf '%s\n' 0.2.0 "$ver" | sort -V | head -n 1)
  [[ $lowest == 0.2.0 ]] || continue
  printf '%s\n' "$tag"
done | sort -V)

if [[ ${#tags[@]} -eq 0 ]]; then
  echo "scripts/registry.sh: no tag v0.2.0 or later" >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
manifest=$work/manifest.json
printf '[\n' > "$manifest"
comma=""
for tag in "${tags[@]}"; do
  version=${tag#v}
  src=$work/src-$version
  built=$work/reg-$version
  mkdir -p "$src"
  git archive "$tag" | tar -x -C "$src"
  formats=$(sed -n 's/^pub const PART_FORMATS: &\[u32\] = &\[\([^]]*\)\];/\1/p' "$src/packages/scene/src/model.rs")
  if [[ -z $formats ]]; then
    echo "scripts/registry.sh: $tag has no PART_FORMATS in packages/scene/src/model.rs" >&2
    exit 1
  fi
  json_formats=$(printf '%s' "$formats" | tr -d ' ')
  "$orbe" registry build "$src" "$built"
  printf '%s{"version":"%s","formats":[%s],"dir":"%s"}' "$comma" "$version" "$json_formats" "$built" >> "$manifest"
  comma=$',\n'
done
printf '\n]\n' >> "$manifest"

cargo build -p orbe-registry --bin publish --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
"$target/debug/publish" "$manifest" "$out"
