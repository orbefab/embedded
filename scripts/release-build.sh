#!/usr/bin/env bash
# Builds a release orbe for one target: the viewer page, then the release binary
# with that page embedded, then a tarball and its sha256. The same script runs
# in CI and on a Mac. It installs nothing.
#
# Usage: scripts/release-build.sh <target> [--out DIR]
#
# <target> is x86_64-unknown-linux-gnu or aarch64-apple-darwin, and it must be
# this machine's rustc host. The tarball is orbe-<version>-<target>.tar.gz and
# holds the orbe binary at its top. The sha256 file sits beside it, in the
# form sha256sum writes. The default output directory is .scratch/release.
# Needs the repo's Node dependencies (pnpm install --frozen-lockfile), Node 24,
# pnpm, cargo and rustc. A tag build refuses a tag that is not v<version>.
# Outside CI, scripts/check-lock.sh takes the shared per-user check lock and
# defaults CARGO_BUILD_JOBS to 2 and NEXTEST_TEST_THREADS to 4.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  cat <<'EOF'
usage: scripts/release-build.sh <target> [--out DIR]

<target>  x86_64-unknown-linux-gnu or aarch64-apple-darwin.
          It must be this machine's rustc host.
--out DIR Directory for the tarball and the sha256 file.
          The default is .scratch/release.
EOF
}

fail() {
  echo "scripts/release-build.sh: $*" >&2
  exit 1
}

orig_args=("$@")
target=
out=
while [[ $# -gt 0 ]]; do
  case $1 in
    -h | --help)
      usage
      exit 0
      ;;
    --out)
      [[ $# -ge 2 ]] || {
        echo "scripts/release-build.sh: --out needs a directory" >&2
        usage >&2
        exit 2
      }
      out=$2
      shift 2
      ;;
    --*)
      echo "scripts/release-build.sh: unknown argument $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -n $target ]]; then
        echo "scripts/release-build.sh: unexpected argument $1" >&2
        usage >&2
        exit 2
      fi
      target=$1
      shift
      ;;
  esac
done

if [[ -z $target ]]; then
  echo "scripts/release-build.sh: pass a target" >&2
  usage >&2
  exit 2
fi
case $target in
  x86_64-unknown-linux-gnu | aarch64-apple-darwin) ;;
  *)
    fail "$target is not a release target. The release targets are x86_64-unknown-linux-gnu and aarch64-apple-darwin."
    ;;
esac
[[ -n $out ]] || out=$PWD/.scratch/release

command -v rustc >/dev/null 2>&1 || fail "need rustc on PATH"
command -v cargo >/dev/null 2>&1 || fail "need cargo on PATH"
command -v pnpm >/dev/null 2>&1 || fail "need pnpm on PATH"
host=$(rustc -vV | sed -n 's/^host: //p')
if [[ $host != "$target" ]]; then
  fail "this machine's rustc host is ${host:-unknown}. Pass that target. This script builds for the machine it runs on."
fi

# shellcheck source=scripts/check-lock.sh
source scripts/check-lock.sh
check_lock_exec "scripts/release-build.sh" "scripts/release-build.sh $target" -- "${orig_args[@]}"

scripts/need-node-deps.sh scripts/release-build.sh
pnpm --filter @orbe/viewer-page build
[[ -f apps/viewer/dist/index.html ]] || fail "apps/viewer/dist/index.html is missing after the viewer build"
marker=$(grep -o 'orbe viewer' apps/viewer/dist/index.html | head -n 1 || true)
[[ -n $marker ]] || fail "the built page has no 'orbe viewer' title to look for in the binary"

cargo build -p orbe --release --locked --target "$target"
target_dir=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
[[ -n $target_dir ]] || fail "cargo metadata printed no target directory"
bin=$target_dir/$target/release/orbe
[[ -x $bin ]] || fail "no binary at $bin"
if ! grep -a -q -F "$marker" "$bin"; then
  fail "the binary at $bin does not contain the viewer page. Build the page before cargo, then build orbe again."
fi

reported=$("$bin" --version)
version=${reported#orbe }
version=${version%% *}
case $version in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) fail "$bin --version printed '$reported', which is not a version" ;;
esac

if [[ ${GITHUB_REF_TYPE:-} == tag ]]; then
  tag=${GITHUB_REF_NAME:-}
  if [[ $tag != "v$version" ]]; then
    fail "the tag is $tag and this orbe is $version. They must match, as v$version."
  fi
fi

name=orbe-$version-$target
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp "$bin" "$stage/orbe"
chmod 755 "$stage/orbe"
mkdir -p "$out"
archive=$out/$name.tar.gz
tar -czf "$archive" -C "$stage" orbe

if command -v sha256sum >/dev/null 2>&1; then
  hash=$(sha256sum "$archive" | awk 'NR==1 { print $1 }')
elif command -v shasum >/dev/null 2>&1; then
  hash=$(shasum -a 256 "$archive" | awk 'NR==1 { print $1 }')
else
  fail "need sha256sum or shasum"
fi
printf '%s  %s\n' "$hash" "$name.tar.gz" >"$archive.sha256"
echo "scripts/release-build.sh: $archive"
echo "scripts/release-build.sh: $archive.sha256"
