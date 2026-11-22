#!/usr/bin/env bash
# Compares two builds of packages/socs/microchip/examples/throughput.rs.
#
# Usage: scripts/compare-throughput.sh [--n N] [--hex PATH] [--cycles N] BIN_A BIN_B
#
# Alternates whole invocations, N of each (default 15). Each invocation is one
# process: the example's own warm-up plus its five timed runs. The script prints
# each side's median of those invocation medians, the range of them, and a
# two-sided Mann-Whitney p. It also prints the 1-minute load, the error count,
# the cycle count, and the pin-edge count.
#
# --hex and --cycles are the example's arguments. With neither, the example
# loads blocks/nano-led-fade/build/fade.ino.hex for 16,000,000 cycles.
#
# Exits 2 when the 1-minute load is above 2, when python3 is missing, or when
# the arguments are wrong. Exits 1 when an invocation fails, its line cannot be
# read, or the invocations disagree on cycles or pin edges. A p below 0.05 is
# a difference. The same binary on both sides shows no difference.
#
# Needs bash and python3. It installs nothing.
set -euo pipefail

cd "$(dirname "$0")/.."

usage() {
  echo "usage: scripts/compare-throughput.sh [--n N] [--hex PATH] [--cycles N] BIN_A BIN_B" >&2
  exit 2
}

if ! command -v python3 >/dev/null; then
  echo "scripts/compare-throughput.sh: needs python3. Install python3 and re-run." >&2
  exit 2
fi

n=15
hex=""
cycles=""
while [[ $# -gt 0 ]]; do
  case $1 in
    --n)
      [[ $# -ge 2 ]] || usage
      n=$2
      shift 2
      ;;
    --hex)
      [[ $# -ge 2 ]] || usage
      hex=$2
      shift 2
      ;;
    --cycles)
      [[ $# -ge 2 ]] || usage
      cycles=$2
      shift 2
      ;;
    --)
      shift
      break
      ;;
    -*)
      usage
      ;;
    *)
      break
      ;;
  esac
done

[[ $# -eq 2 ]] || usage
bin_a=$1
bin_b=$2

if [[ ! $n =~ ^[1-9][0-9]*$ ]]; then
  echo "scripts/compare-throughput.sh: --n must be a whole number above 0" >&2
  exit 2
fi
if [[ -n $cycles && -z $hex ]]; then
  echo "scripts/compare-throughput.sh: --cycles needs --hex" >&2
  exit 2
fi
if [[ -n $cycles && ! $cycles =~ ^[1-9][0-9]*$ ]]; then
  echo "scripts/compare-throughput.sh: --cycles must be a whole number above 0" >&2
  exit 2
fi
for bin in "$bin_a" "$bin_b"; do
  if [[ ! -x $bin ]]; then
    echo "scripts/compare-throughput.sh: $bin is not an executable file" >&2
    exit 2
  fi
done

load=$(awk '{print $1}' /proc/loadavg)
if awk -v load="$load" 'BEGIN { exit (load > 2) ? 0 : 1 }'; then
  echo "scripts/compare-throughput.sh: 1-minute load is ${load}, above 2. Retry when the machine is quiet." >&2
  exit 2
fi
echo "compare: load before ${load}"

args=()
if [[ -n $hex ]]; then
  args+=("$hex")
  if [[ -n $cycles ]]; then
    args+=("$cycles")
  fi
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

run_one() {
  local side=$1
  local bin=$2
  local out status
  set +e
  out=$("$bin" "${args[@]}" 2>"$tmp/err")
  status=$?
  set -e
  if [[ $status -ne 0 ]]; then
    echo "compare: ${side} invocation failed (exit ${status})" >&2
    cat "$tmp/err" >&2
    printf '%s\n' "$out" >&2
    exit 1
  fi
  local ms edges work
  ms=$(sed -n 's/.*median \([0-9.][0-9.]*\) ms per simulated second.*/\1/p' <<<"$out" | tail -1)
  edges=$(sed -n 's/.* \([0-9][0-9]*\) pin edges.*/\1/p' <<<"$out" | tail -1)
  work=$(sed -n 's/.* \([0-9][0-9]*\) cycles,.*/\1/p' <<<"$out" | tail -1)
  if [[ -z $ms || -z $edges || -z $work ]]; then
    echo "compare: ${side} invocation printed no throughput line" >&2
    printf '%s\n' "$out" >&2
    exit 1
  fi
  printf '%s %s %s\n' "$ms" "$edges" "$work" >>"$tmp/$side"
}

i=0
while [[ $i -lt $n ]]; do
  run_one a "$bin_a"
  run_one b "$bin_b"
  i=$((i + 1))
done

load_after=$(awk '{print $1}' /proc/loadavg)
python3 - "$tmp/a" "$tmp/b" "$n" "$load_after" <<'PY'
import math
import sys


def erf(x):
    # Abramowitz and Stegun 7.1.26, absolute error under 1.5e-7.
    sign = 1 if x >= 0 else -1
    x = abs(x)
    t = 1 / (1 + 0.3275911 * x)
    y = 1 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * math.exp(-x * x)
    return sign * y


def phi_complement(z):
    return 0.5 * (1 - erf(z / math.sqrt(2)))


def median(values):
    ordered = sorted(values)
    count = len(ordered)
    mid = count // 2
    if count % 2 == 1:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2


def read_side(path):
    rows = []
    with open(path) as handle:
        for line in handle:
            ms, edges, work = line.split()
            rows.append((float(ms), int(edges), int(work)))
    return rows


def mann_whitney(left, right):
    """Two-sided p from the normal approximation with tie correction."""
    n_a = len(left)
    n_b = len(right)
    pooled = [(value, 0) for value in left] + [(value, 1) for value in right]
    pooled.sort(key=lambda item: item[0])
    ranks = [0.0] * len(pooled)
    tie_sum = 0
    index = 0
    while index < len(pooled):
        end = index
        while end < len(pooled) and pooled[end][0] == pooled[index][0]:
            end += 1
        # Ranks are 1-based. Ties share the average rank.
        average = (index + 1 + end) / 2
        for at in range(index, end):
            ranks[at] = average
        length = end - index
        tie_sum += length**3 - length
        index = end
    rank_a = sum(rank for rank, item in zip(ranks, pooled) if item[1] == 0)
    u_a = n_a * n_b + n_a * (n_a + 1) / 2 - rank_a
    u_b = n_a * n_b - u_a
    u = min(u_a, u_b)
    total = n_a + n_b
    mu = n_a * n_b / 2
    if total < 2:
        return u, 1.0
    variance = n_a * n_b / 12 * ((total + 1) - tie_sum / (total * (total - 1)))
    if variance <= 0:
        return u, 1.0
    sigma = math.sqrt(variance)
    z = (abs(u - mu) - 0.5) / sigma
    if z < 0:
        z = 0.0
    p = min(1.0, 2 * phi_complement(z))
    return u, p


def fmt_p(p):
    if p < 1e-4:
        return f"{p:.2e}"
    return f"{p:.4f}"


left = read_side(sys.argv[1])
right = read_side(sys.argv[2])
expected = int(sys.argv[3])
load_after = sys.argv[4]
if len(left) != expected or len(right) != expected:
    print(f"compare: expected {expected} samples a side, got {len(left)} and {len(right)}", file=sys.stderr)
    sys.exit(1)

edges = {row[1] for row in left + right}
works = {row[2] for row in left + right}
ms_a = [row[0] for row in left]
ms_b = [row[0] for row in right]
for name, samples in ("A", ms_a), ("B", ms_b):
    print(
        f"compare: {name} median {median(samples):.2f} ms, range {min(samples):.2f} to {max(samples):.2f}, n={len(samples)}"
    )
    print("compare: " + name + " samples " + " ".join(f"{sample:.1f}" for sample in samples))

u, p = mann_whitney(ms_a, ms_b)
print(f"compare: Mann-Whitney two-sided p={fmt_p(p)} U={u:.1f} (normal approximation, tie correction)")
print(f"compare: load after {load_after}")
if len(works) != 1 or len(edges) != 1:
    cycles_text = " ".join(str(item) for item in sorted(works))
    edges_text = " ".join(str(item) for item in sorted(edges))
    print(f"compare: invocations disagree, cycles {cycles_text}, pin edges {edges_text}, errors 0")
    sys.exit(1)
print(f"compare: cycles {next(iter(works))}, pin edges {next(iter(edges))}, errors 0")
print("compare: difference" if p < 0.05 else "compare: no difference")
PY
