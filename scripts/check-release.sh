#!/usr/bin/env bash
# Runs a newcomer's first path on a release orbe. Pass the orbe binary, or the
# archive scripts/release-build.sh wrote. An archive is checked against the
# sha256 file beside it, then unpacked. The commands run in an empty temp
# folder, and the script stops at the first one that fails. It removes that
# folder when it exits.
#
# The same script runs on Linux and on macOS. It uses mktemp with a template,
# and sha256sum or shasum -a 256. It needs bash and curl or wget.
#
# Usage: scripts/check-release.sh <orbe-or-archive>
#
# `show --no-browser` prints the page URL and opens no browser. The script
# fetches that page and the module script it names. A .wasm file named by the
# page or that script is fetched too, and must be WebAssembly. The page built
# into orbe is HTML and one script. It names no wasm file: the browser package
# loads for a Block page, not for this session.
set -euo pipefail

usage() {
  cat <<'EOF'
usage: scripts/check-release.sh <orbe-or-archive>

<orbe-or-archive>  The orbe binary, or an orbe-<version>-<target>.tar.gz
                   archive. The archive's sha256 file is the same path
                   with .sha256 added, as scripts/release-build.sh writes it.
EOF
}

fail() {
  echo "scripts/check-release.sh: $*" >&2
  exit 1
}

usage_error() {
  echo "scripts/check-release.sh: $*" >&2
  usage >&2
  exit 2
}

work=
project=
orbe_bin=

cleanup() {
  status=$?
  if [[ -n $orbe_bin && -d $project ]]; then
    (cd "$project" && "$orbe_bin" close) >/dev/null 2>&1 || true
  fi
  if [[ -n $work ]]; then
    rm -rf "$work" || true
  fi
  exit "$status"
}

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk 'NR==1 { print $1 }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk 'NR==1 { print $1 }'
  else
    fail "need sha256sum or shasum to check the archive"
  fi
}

# Prints the command, keeps its stdout, and stops when it exits non-zero.
capture() {
  echo "scripts/check-release.sh: $*" >&2
  local out
  out=$("$@") && true
  status=$?
  if [[ $status -ne 0 ]]; then
    printf '%s\n' "$out" >&2
    echo "scripts/check-release.sh: failed: $* (exit $status)" >&2
    exit 1
  fi
  printf '%s\n' "$out"
}

capture_in() {
  echo "scripts/check-release.sh: $*  [$project]" >&2
  local out
  out=$(cd "$project" && "$@") && true
  status=$?
  if [[ $status -ne 0 ]]; then
    printf '%s\n' "$out" >&2
    echo "scripts/check-release.sh: failed: $* (exit $status)" >&2
    exit 1
  fi
  printf '%s\n' "$out"
}

run_in() {
  echo "scripts/check-release.sh: $*  [$project]" >&2
  if ! (cd "$project" && "$@"); then
    echo "scripts/check-release.sh: failed: $*" >&2
    exit 1
  fi
}

# A URL on the viewer origin, with a relative ref resolved against base.
resolve_url() {
  local base=$1 ref=$2 origin
  ref=${ref%%\?*}
  ref=${ref%%#*}
  case $ref in
    *..*) fail "refusing $ref" ;;
  esac
  case $ref in
    http://*)
      printf '%s\n' "$ref"
      ;;
    /*)
      origin=${base#http://}
      origin=${origin%%/*}
      printf 'http://%s%s\n' "$origin" "$ref"
      ;;
    *)
      base=${base%/*}/
      ref=${ref#./}
      printf '%s%s\n' "$base" "$ref"
      ;;
  esac
}

fetch() {
  local url=$1 dest=$2
  echo "scripts/check-release.sh: fetch $url" >&2
  if command -v curl >/dev/null 2>&1; then
    # A proxy would send the loopback URL off this machine.
    if ! env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u all_proxy \
      curl --noproxy '*' -fsS --retry 2 --retry-delay 1 --max-time 60 -o "$dest" "$url"; then
      fail "could not fetch $url"
    fi
  elif command -v wget >/dev/null 2>&1; then
    if ! env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u all_proxy \
      wget -q -O "$dest" "$url"; then
      fail "could not fetch $url"
    fi
  else
    fail "need curl or wget to fetch the viewer page"
  fi
  [[ -s $dest ]] || fail "$url was empty"
}

page_answers() {
  local url=$1
  if command -v curl >/dev/null 2>&1; then
    env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u all_proxy \
      curl --noproxy '*' -s -o /dev/null --max-time 2 "$url" >/dev/null 2>&1
  else
    env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u all_proxy \
      wget -q -O /dev/null "$url" >/dev/null 2>&1
  fi
}

input=
while [[ $# -gt 0 ]]; do
  case $1 in
    -h | --help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      usage_error "unknown option $1"
      ;;
    *)
      if [[ -n $input ]]; then
        usage_error "unexpected argument $1"
      fi
      input=$1
      shift
      ;;
  esac
done
[[ $# -eq 0 ]] || usage_error "unexpected argument $1"
[[ -n $input ]] || usage_error "pass the orbe binary or an archive"

case $input in
  /*) ;;
  *) input=$PWD/$input ;;
esac
[[ -f $input ]] || fail "no file at $input"

work=$(mktemp -d "${TMPDIR:-/tmp}/orbe-check-release.XXXXXX")
trap cleanup EXIT

want_version=
if [[ $input == *.tar.gz ]]; then
  sumfile=$input.sha256
  [[ -f $sumfile ]] || fail "no sha256 file at $sumfile"
  base=$(basename "$input")
  stem=${base%.tar.gz}
  case $stem in
    orbe-*-x86_64-unknown-linux-gnu)
      want_version=${stem#orbe-}
      want_version=${want_version%-x86_64-unknown-linux-gnu}
      ;;
    orbe-*-aarch64-apple-darwin)
      want_version=${stem#orbe-}
      want_version=${want_version%-aarch64-apple-darwin}
      ;;
    *)
      fail "archive name $base is not orbe-<version>-<target>.tar.gz"
      ;;
  esac
  case $want_version in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) fail "archive name $base has no version" ;;
  esac

  want=$(awk 'NR==1 { print $1 }' "$sumfile" | tr 'A-F' 'a-f')
  case $want in
    *[!0-9a-f]*) fail "the checksum in $sumfile is not hex" ;;
  esac
  [[ ${#want} -eq 64 ]] || fail "the checksum in $sumfile is not 64 hex digits"
  named=$(awk 'NR==1 { print $2 }' "$sumfile")
  [[ $named == "$base" ]] || fail "the checksum file names $named, not $base"
  got=$(hash_file "$input" | tr 'A-F' 'a-f')
  [[ $want == "$got" ]] || fail "sha256 mismatch: the archive has $got and $sumfile says $want"

  tar -tzf "$input" >"$work/list" || fail "could not list the archive"
  while IFS= read -r entry || [[ -n $entry ]]; do
    case $entry in
      "") continue ;;
      /*) fail "archive entry $entry starts at the root" ;;
      *..*) fail "archive entry $entry leaves its folder" ;;
    esac
  done <"$work/list"
  mkdir -p "$work/root"
  tar -xzf "$input" -C "$work/root" || fail "could not extract the archive"
  [[ -f $work/root/orbe ]] || fail "the archive has no orbe binary at its top"
  chmod 755 "$work/root/orbe" || fail "could not mark the unpacked orbe executable"
  orbe_bin=$work/root/orbe
else
  [[ -x $input ]] || fail "$input is not executable"
  orbe_bin=$input
fi

reported=$(capture "$orbe_bin" --version)
printf '%s\n' "$reported"
case $reported in
  "orbe "[0-9]*) ;;
  *) fail "orbe --version printed '$reported', which is not a version" ;;
esac
got_version=${reported#orbe }
got_version=${got_version%%[[:space:]]*}
if [[ -n $want_version && $got_version != "$want_version" ]]; then
  fail "orbe --version is $got_version, and the archive name says $want_version"
fi

skill=$(capture "$orbe_bin" skills get core)
[[ -n $skill ]] || fail "orbe skills get core printed nothing"
echo "scripts/check-release.sh: skills get core printed ${#skill} bytes" >&2

project=$work/project
mkdir -p "$project"

run_in "$orbe_bin" add nano-button-led
run_in "$orbe_bin" check scene.jsonc
run_in "$orbe_bin" lint scene.jsonc
run_in "$orbe_bin" test

opened=$(capture_in "$orbe_bin" open)
printf '%s\n' "$opened"
case $opened in
  opened\ *) ;;
  *) fail "orbe open did not open the scene: $opened" ;;
esac
run_in "$orbe_bin" step 100ms
reading=$(capture_in "$orbe_bin" get @led1)
printf '%s\n' "$reading"
[[ -n $reading ]] || fail "orbe get @led1 printed nothing"
closed=$(capture_in "$orbe_bin" close)
printf '%s\n' "$closed"
case $closed in
  *"closed at t="*) ;;
  *) fail "orbe close did not say the session closed: $closed" ;;
esac

# show serves the page for the session open just above, so open again after
# that close. The close after the fetch is what stops the server.
opened=$(capture_in "$orbe_bin" open)
printf '%s\n' "$opened"
case $opened in
  opened\ *) ;;
  *) fail "orbe open did not open the scene: $opened" ;;
esac
shown=$(capture_in "$orbe_bin" show --no-browser)
printf '%s\n' "$shown"
url=$(printf '%s\n' "$shown" | awk 'match($0, /http:\/\/127\.0\.0\.1:[0-9]+\/#t=[0-9a-f]+/) { print substr($0, RSTART, RLENGTH); exit }')
[[ -n $url ]] || fail "orbe show --no-browser printed no page URL: $shown"
origin=${url%%#*}
origin=${origin%/}
page=$origin/

fetch "$page" "$work/page.html"
grep -q 'orbe viewer' "$work/page.html" || fail "the page at $page has no 'orbe viewer' title"
echo "scripts/check-release.sh: page loaded" >&2

script_n=0
while IFS= read -r src || [[ -n $src ]]; do
  [[ -n $src ]] || continue
  resolved=$(resolve_url "$page" "$src")
  case $resolved in
    "$origin"/*) ;;
    *) fail "the page names $src, which is not on $origin" ;;
  esac
  fetch "$resolved" "$work/script-$script_n"
  grep -q 'orbeViewer' "$work/script-$script_n" || fail "$resolved is not the viewer script"
  script_n=$((script_n + 1))
done <<EOF
$(sed -n 's/.*src="\([^"]*\)".*/\1/p' "$work/page.html")
EOF
[[ $script_n -gt 0 ]] || fail "the page names no script"
echo "scripts/check-release.sh: script loaded" >&2

: >"$work/wasm-refs"
scan_wasm() {
  local file=$1 base=$2 ref resolved
  while IFS= read -r ref || [[ -n $ref ]]; do
    [[ -n $ref ]] || continue
    resolved=$(resolve_url "$base" "$ref")
    case $resolved in
      "$origin"/*) ;;
      *) fail "the page names $ref, which is not on $origin" ;;
    esac
    printf '%s\n' "$resolved"
  done <<EOF
$(grep -oE '[A-Za-z0-9_./+-]+\.wasm' "$file" || true)
EOF
}
scan_wasm "$work/page.html" "$page" >>"$work/wasm-refs"
i=0
while [[ $i -lt $script_n ]]; do
  script_url=$(sed -n 's/.*src="\([^"]*\)".*/\1/p' "$work/page.html" | awk -v n="$((i + 1))" 'NR==n { print; exit }')
  script_url=$(resolve_url "$page" "$script_url")
  scan_wasm "$work/script-$i" "$script_url" >>"$work/wasm-refs"
  i=$((i + 1))
done

wasm_n=0
if [[ -s $work/wasm-refs ]]; then
  sort -u "$work/wasm-refs" -o "$work/wasm-refs"
  while IFS= read -r wasm_url || [[ -n $wasm_url ]]; do
    [[ -n $wasm_url ]] || continue
    fetch "$wasm_url" "$work/wasm-$wasm_n"
    magic=$(od -An -tx1 -N 4 "$work/wasm-$wasm_n" | tr -d ' \n' | tr 'A-F' 'a-f')
    [[ $magic == 0061736d ]] || fail "$wasm_url is not WebAssembly (first bytes $magic)"
    wasm_n=$((wasm_n + 1))
  done <"$work/wasm-refs"
fi
if [[ $wasm_n -eq 0 ]]; then
  echo "scripts/check-release.sh: the page names no wasm file" >&2
else
  echo "scripts/check-release.sh: wasm loaded ($wasm_n)" >&2
fi

closed=$(capture_in "$orbe_bin" close)
printf '%s\n' "$closed"
case $closed in
  *"closed at t="*) ;;
  *) fail "orbe close did not say the session closed: $closed" ;;
esac

stopped=0
tries=0
while [[ $tries -lt 20 ]]; do
  if ! page_answers "$page"; then
    stopped=1
    break
  fi
  tries=$((tries + 1))
  sleep 0.1
done
[[ $stopped -eq 1 ]] || fail "the viewer server still answered after orbe close"
echo "scripts/check-release.sh: the viewer server stopped" >&2
echo "scripts/check-release.sh: ok" >&2
