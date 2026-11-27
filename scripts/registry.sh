#!/usr/bin/env bash
# Builds the registry the site serves from release tags v0.2.0 and later.
# An older tag's record at id@version stays. The newest tag supplies the index
# name and the schemas. Each index row names the lowest orbe whose part formats
# include that record's format and the formats of every part it depends on,
# all the way down. A format no tag reads names the patch after the newest tag.
#
# The working tree is built too, as the next release, the patch after the newest
# tag. A part or Block it changes at a version a tag published fails the run,
# since the site keeps serving the tag's bytes at that version. --with-tree also
# merges the working tree into <out>, as the next release would serve it.
#
# usage: scripts/registry.sh [--with-tree] <orbe> <out>
set -euo pipefail
cd "$(dirname "$0")/.."

with_tree=()
if [[ ${1:-} == --with-tree ]]; then
  with_tree=(--with-tree)
  shift
fi
if [[ $# -ne 2 ]]; then
  echo "usage: scripts/registry.sh [--with-tree] <orbe> <out>" >&2
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

# formats <src> <name>: PART_FORMATS from that source, as a JSON list body.
formats() {
  local found
  found=$(sed -n 's/^pub const PART_FORMATS: &\[u32\] = &\[\([^]]*\)\];/\1/p' "$1/packages/scene/src/model.rs")
  if [[ -z $found ]]; then
    echo "scripts/registry.sh: $2 has no PART_FORMATS in packages/scene/src/model.rs" >&2
    exit 1
  fi
  printf '%s' "$found" | tr -d ' '
}

for tag in "${tags[@]}"; do
  version=${tag#v}
  src=$work/src-$version
  built=$work/reg-$version
  mkdir -p "$src"
  git archive "$tag" | tar -x -C "$src"
  json_formats=$(formats "$src" "$tag")
  "$orbe" registry build "$src" "$built"
  printf '%s{"version":"%s","formats":[%s],"dir":"%s"}' "$comma" "$version" "$json_formats" "$built" >> "$manifest"
  comma=$',\n'
done

newest=${tags[-1]#v}
next="${newest%.*}.$(( ${newest##*.} + 1 ))"
tree_formats=$(formats . "the working tree")
"$orbe" registry build . "$work/reg-tree"
printf '%s{"version":"%s","formats":[%s],"dir":"%s","tree":true}' "$comma" "$next" "$tree_formats" "$work/reg-tree" >> "$manifest"
printf '\n]\n' >> "$manifest"

cargo build -p orbe-registry --bin publish --locked
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
"$target/debug/publish" "${with_tree[@]}" "$manifest" "$out"
