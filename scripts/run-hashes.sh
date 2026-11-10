#!/usr/bin/env bash
# Writes a manifest of every engine run this repo checks.
#
# Usage: scripts/run-hashes.sh [--time]
#
# Runs each netlist in packages/engine/tests/circuits. Prints one line per
# run-folder file, `sha256  path`. A manifest.json line is the hash of its JSON
# with created_at removed, which is how packages/engine/scripts/wasi-check.sh
# compares two manifests. Every other file is hashed as written. The lines are
# sorted by path, so one circuit's files sit together.
#
# Each Block adds its resolved netlist, a short run of that netlist, and the
# stdout of `orbe test -v`. `blocks/<id>/netlist.json` is the netlist v0 its
# scene resolves to. `blocks/<id>/run/` is every file of that netlist run after
# analysis.transient.stop_ps is set to 2000000000000, which is 2 s. The scene
# stops at 7200 s on a 1 ms grid, and orbe-run rejects that trace, so the
# manifest hashes this 2 s run instead. `blocks/<id>/orbe-test` is the stdout
# of `orbe test -v` on that Block, which includes each expect's measured value.
#
# A run that does not exit 0 stops the script. It names the circuit and the
# exit code, prints the runner's report, and exits with that code. A Block
# test that fails prints its output and exits with the test's status.
# Paths are relative to the repo root. Stderr names each circuit and Block as it starts.
#
# --time adds, after those lines, `time circuits/<name> <milliseconds>` for
# each circuit. The number is the median wall time of five runs of that
# netlist. Leave --time off when you diff two manifests.
#
# RUNNER is the binary that accepts `run <netlist> --out <dir> [--blobs <dir>]`.
# When RUNNER is unset, the script builds release orbe-run. Point RUNNER at
# target/release/sim to hash that binary's folders instead.
#
# The script needs bash, python3, sha256sum and cargo. It installs nothing.
set -euo pipefail
cd "$(dirname "$0")/.."
export LC_ALL=C
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

usage() {
  echo "usage: scripts/run-hashes.sh [--time]" >&2
  exit 2
}

need() {
  local tool=$1 hint=$2
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "run-hashes: $tool is not installed. $hint" >&2
    exit 1
  fi
}

mask_manifest() {
  python3 - "$1" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    doc = json.load(handle)
doc.pop("created_at", None)
sys.stdout.write(json.dumps(doc, separators=(",", ":"), ensure_ascii=False))
PY
}

# A Block scene stops at 7200 s. orbe-run rejects that trace, so the hashed run
# is the same netlist with its stop cut to 2 s. The bytes other than stop_ps stay.
cut_stop() {
  python3 - "$1" "$2" <<'PY'
import json
import re
import sys

src, dest = sys.argv[1], sys.argv[2]
stop = 2_000_000_000_000
with open(src, encoding="utf-8") as handle:
    text = handle.read()
doc = json.loads(text)
try:
    doc["analysis"]["transient"]["stop_ps"] = stop
except (KeyError, TypeError):
    sys.stderr.write(f"run-hashes: {src} has no analysis.transient.stop_ps\n")
    sys.exit(1)
rewritten, count = re.subn(
    r'("stop_ps"\s*:\s*)-?\d+',
    lambda match: match.group(1) + str(stop),
    text,
    count=1,
)
if count != 1 or json.loads(rewritten) != doc:
    sys.stderr.write(f"run-hashes: could not set stop_ps in {src}\n")
    sys.exit(1)
with open(dest, "w", encoding="utf-8") as handle:
    handle.write(rewritten)
PY
}

hash_tree() {
  local dir=$1 prefix=$2 file rel digest
  while IFS= read -r file; do
    rel=${file#"$dir"/}
    if [[ ${rel##*/} == manifest.json ]]; then
      digest=$(mask_manifest "$file" | sha256sum)
    else
      digest=$(sha256sum "$file")
    fi
    digest=${digest%% *}
    printf '%s  %s\n' "$digest" "$prefix/$rel"
  done < <(find "$dir" -type f -print | LC_ALL=C sort)
}

run_netlist() {
  local label=$1 netlist=$2 out=$3 code
  rm -rf "$out"
  set +e
  "$RUNNER" run "$netlist" --out "$out" --blobs "$blobs" >"$scratch/report.json"
  code=$?
  set -e
  if ((code != 0)); then
    echo "run-hashes: $label exited $code" >&2
    cat "$scratch/report.json" >&2
    exit "$code"
  fi
  if [[ ! -f $out/manifest.json ]]; then
    echo "run-hashes: $label wrote no manifest.json" >&2
    exit 1
  fi
}

median_ms() {
  local -a sorted
  local mid
  mapfile -t sorted < <(printf '%s\n' "$@" | LC_ALL=C sort -n)
  mid=${sorted[$((${#sorted[@]} / 2))]}
  awk -v n="$mid" 'BEGIN { printf "%.3f", n / 1000000 }'
}

time_flag=0
case $# in
  0) ;;
  1)
    [[ $1 == --time ]] || usage
    time_flag=1
    ;;
  *) usage ;;
esac

need python3 "Install it (apt install python3) and re-run."
need sha256sum "Install it (apt install coreutils) and re-run."
need cargo "Install Rust and re-run."
if [[ $time_flag -eq 1 ]] && ! date +%s%N >/dev/null 2>&1; then
  echo "run-hashes: date cannot print nanoseconds, so --time cannot time a run." >&2
  exit 1
fi

scratch=$(mktemp -d "${TMPDIR:-/tmp}/run-hashes.XXXXXX")
lines=$(mktemp)
times=$(mktemp)
trap 'rm -rf "$scratch" "$lines" "$times"' EXIT

blobs=$(packages/engine/scripts/blobs.sh)
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
if [[ -z $target ]]; then
  echo "run-hashes: cargo metadata did not name a target directory" >&2
  exit 1
fi

if [[ -z ${RUNNER:-} ]]; then
  cargo build -q -p orbe-circuit --features runner --release --locked --bin orbe-run
  RUNNER=$target/release/orbe-run
fi
if [[ ! -x $RUNNER ]]; then
  echo "run-hashes: RUNNER is not an executable ($RUNNER)" >&2
  exit 1
fi
cargo build -q -p orbe --release --locked
cargo build -q -p orbe-scene --release --locked --example netlist
orbe=$target/release/orbe
resolve=$target/release/examples/netlist

for netlist in packages/engine/tests/circuits/*/netlist.json; do
  name=$(basename "$(dirname "$netlist")")
  echo "run-hashes: circuits/$name" >&2
  out=$scratch/run
  if [[ $time_flag -eq 1 ]]; then
    samples=()
    for _ in 1 2 3 4 5; do
      start=$(date +%s%N)
      run_netlist "$netlist" "$netlist" "$out"
      end=$(date +%s%N)
      samples+=("$((end - start))")
    done
    printf 'time circuits/%s %s\n' "$name" "$(median_ms "${samples[@]}")" >>"$times"
  else
    run_netlist "$netlist" "$netlist" "$out"
  fi
  hash_tree "$out" "circuits/$name" >>"$lines"
done

for scene in blocks/*/scene.jsonc; do
  name=$(basename "$(dirname "$scene")")
  echo "run-hashes: blocks/$name" >&2
  "$resolve" "$scene" >"$scratch/netlist.json"
  digest=$(sha256sum "$scratch/netlist.json")
  digest=${digest%% *}
  printf '%s  %s\n' "$digest" "blocks/$name/netlist.json" >>"$lines"
  cut_stop "$scratch/netlist.json" "$scratch/short.json"
  run_netlist "blocks/$name" "$scratch/short.json" "$scratch/run"
  hash_tree "$scratch/run" "blocks/$name/run" >>"$lines"
  set +e
  "$orbe" test -v "blocks/$name" >"$scratch/test.out"
  code=$?
  set -e
  if ((code != 0)); then
    echo "run-hashes: blocks/$name orbe test exited $code" >&2
    cat "$scratch/test.out" >&2
    exit "$code"
  fi
  digest=$(sha256sum "$scratch/test.out")
  digest=${digest%% *}
  printf '%s  %s\n' "$digest" "blocks/$name/orbe-test" >>"$lines"
done

LC_ALL=C sort -k2 -o "$lines" "$lines"
cat "$lines"
if [[ $time_flag -eq 1 ]]; then
  LC_ALL=C sort -o "$times" "$times"
  cat "$times"
fi
