#!/usr/bin/env bash
# Compares ELF diagnostics from two checkouts.
#
# Usage: scripts/elf-corpus.sh <left-root> <right-root>
#
# Writes one line per input for the Xtensa executable loader, the AVR image loader
# and the ESP32-S3 ROM loader. Inputs are the malformed files this script builds,
# every *.elf under packages/cores/xtensa/tests/fixtures and
# packages/socs/espressif/tests/fixtures, each of those with one byte flipped
# (xor 0xff) per byte, and the pinned ROM loaded once when the blob cache holds it.
# Prints the input count, then either "diff empty" or each distinct diagnostic
# change with its count and one example. The full line diff is on stdout after that
# when it is under 200 lines, and always in the path named by ELF_CORPUS_DIFF.
#
# The ROM loader is crate-private, so the script copies scripts/elf-corpus/rom_probe.rs
# into each tree for the run and removes it. A tree with other changes in
# packages/socs/espressif/src/s3/mod.rs is restored with git checkout of that file.
#
# The script needs bash, python3, cargo and diff. It installs nothing.
set -euo pipefail
cd "$(dirname "$0")/.."
here=$PWD

if [[ $# -ne 2 ]]; then
  echo "usage: scripts/elf-corpus.sh <left-root> <right-root>" >&2
  exit 2
fi
left=$(cd "$1" && pwd)
right=$(cd "$2" && pwd)

work=$(mktemp -d "${TMPDIR:-/tmp}/elf-corpus.XXXXXX")
restore_trees() {
  rm -rf "$work"
  local root
  for root in "$left" "$right"; do
    git -C "$root" checkout -- packages/socs/espressif/src/s3/mod.rs 2>/dev/null || true
    rm -f "$root/packages/socs/espressif/src/s3/rom_probe.rs"
  done
}
trap restore_trees EXIT
python3 scripts/elf-corpus/cases.py "$work/cases"
mkdir -p "$work/fixtures"
find packages/cores/xtensa/tests/fixtures packages/socs/espressif/tests/fixtures -name '*.elf' -exec cp {} "$work/fixtures/" \;
rom=${ELF_CORPUS_ROM:-${ORBE_BLOBS:-${XDG_CACHE_HOME:-$HOME/.cache}/orbe/blobs}/c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd}
if [[ ! -f $rom ]]; then
  rom=
fi

run_one() {
  local root=$1 out=$2 target
  target=$work/target-$(basename "$root")
  if [[ $root == "$here" ]]; then
    target=$here/.scratch/target-head
  fi
  local probe=$work/probe-$out
  mkdir -p "$probe/src"
  cp scripts/elf-corpus/main.rs "$probe/src/main.rs"
  cat >"$probe/Cargo.toml" <<EOF
[package]
name = "elf-corpus"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
orbe-engine = { path = "$root/packages/engine" }
orbe-microchip = { path = "$root/packages/socs/microchip" }
sha2 = "0.10"
EOF
  local rom_args=()
  if [[ -n $rom ]]; then
    rom_args=("$rom")
  fi
  CARGO_TARGET_DIR=$target cargo run -q --manifest-path "$probe/Cargo.toml" --release -- "$work/cases" "$work/fixtures" "${rom_args[@]}" >"$work/$out.ea"
  cp scripts/elf-corpus/rom_probe.rs "$root/packages/socs/espressif/src/s3/rom_probe.rs"
  python3 - "$root/packages/socs/espressif/src/s3/mod.rs" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
old = "mod rom;\n"
new = "mod rom;\n#[cfg(test)]\nmod rom_probe;\n"
if "mod rom_probe;" not in text:
    if old not in text:
        sys.exit("elf-corpus: s3/mod.rs has no `mod rom;`")
    path.write_text(text.replace(old, new, 1))
PY
  set +e
  ELF_CORPUS_DIR=$work/cases ELF_CORPUS_FIXTURES=$work/fixtures ELF_CORPUS_ROM=${rom:-} \
    CARGO_TARGET_DIR=$target cargo test -q -p orbe-espressif --lib --release --manifest-path "$root/Cargo.toml" \
    rom_probe::corpus -- --nocapture >"$work/$out.rom"
  local code=$?
  set -e
  git -C "$root" checkout -- packages/socs/espressif/src/s3/mod.rs
  rm -f "$root/packages/socs/espressif/src/s3/rom_probe.rs"
  if ((code != 0)); then
    echo "elf-corpus: ROM probe failed in $root" >&2
    cat "$work/$out.rom" >&2
    exit "$code"
  fi
  grep '^engine \|^avr ' "$work/$out.ea" >"$work/$out.ea.lines"
  grep '^rom ' "$work/$out.rom" >"$work/$out.rom.lines"
  if [[ ! -s $work/$out.rom.lines ]]; then
    echo "elf-corpus: ROM probe printed no lines in $root" >&2
    cat "$work/$out.rom" >&2
    exit 1
  fi
  cat "$work/$out.ea.lines" "$work/$out.rom.lines" | LC_ALL=C sort >"$work/$out.lines"
}

run_one "$left" left
run_one "$right" right
inputs=$(grep -c . "$work/left.lines" || true)
diff_out=${ELF_CORPUS_DIFF:-$work/diff}
diff -u "$work/left.lines" "$work/right.lines" >"$diff_out" || true
echo "inputs $inputs"
python3 - "$work/left.lines" "$work/right.lines" <<'PY'
import sys
from collections import Counter

left = open(sys.argv[1], encoding="utf-8").read().splitlines()
right = open(sys.argv[2], encoding="utf-8").read().splitlines()
if len(left) != len(right):
    sys.stdout.write(f"line counts {len(left)} {len(right)}\n")

def key(line):
    parts = line.split(" ", 2)
    return " ".join(parts[:2])

lk = {key(line): line for line in left}
rk = {key(line): line for line in right}
missing = sorted(set(lk) ^ set(rk))
if missing:
    sys.stdout.write(f"keys only on one side {len(missing)}\n")
    for item in missing[:20]:
        sys.stdout.write(f"key {item}\n")
groups = Counter()
examples = {}
members = {}
same = 0
for name in sorted(set(lk) & set(rk)):
    if lk[name] == rk[name]:
        same += 1
        continue
    change = lk[name].split(" ", 2)[-1] + " -> " + rk[name].split(" ", 2)[-1]
    groups[change] += 1
    examples.setdefault(change, name)
    members.setdefault(change, []).append(name)
sys.stdout.write(f"same {same}\n")
sys.stdout.write(f"differ {sum(groups.values())}\n")
if not groups:
    sys.stdout.write("diff empty\n")
for change, count in groups.most_common():
    sys.stdout.write(f"{count} {examples[change]} {change}\n")
    for name in members[change]:
        sys.stdout.write(f"  {name}\n")
PY
diff_lines=$(grep -c . "$diff_out" || true)
if ((diff_lines > 0 && diff_lines < 200)); then
  cat "$diff_out"
fi
echo "full diff $diff_out"
