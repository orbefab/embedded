#!/usr/bin/env bash
# Runs a newcomer's first path on a release orbe. Pass the orbe binary, or the
# archive scripts/release-build.sh wrote. An archive is checked against the
# sha256 file beside it, then unpacked. `sh scripts/install.sh --tarball`
# installs that archive into the temp folder. It downloads nothing. A checksum
# that does not match is refused, and the installed `orbe --version` must be
# the version in the archive name. The commands then run on that installed
# binary. They run in an empty temp folder, and the script stops at the first
# one that fails. It removes that folder when it exits.
#
# The same script runs on Linux and on macOS. It uses mktemp with a template,
# and sha256sum or shasum -a 256. It needs bash and curl. wget can fetch the
# page when curl is absent, and the session check still needs curl.
#
# Usage: scripts/check-release.sh <orbe-or-archive>
#
# `show --no-browser` prints the page URL and opens no browser. The script
# fetches that page and the script it names. The token in the URL's fragment
# is the session: the viewer opens ws://<host>/session?t=<token>, then loads
# each part model from /parts/<id>/<id>.glb. The script checks that upgrade,
# that a wrong token is refused, and that one model comes back as glTF.
#
# Each orbe command has a deadline. macOS has no timeout(1), so the wait is a
# bash loop that kills the command when the time is up. Stopping this script
# stops that command too, then closes the session and removes the work folder.
set -euo pipefail

scripts_dir=$(cd "$(dirname "$0")" && pwd)

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
# The orbe command this script is waiting on, when one is running.
cmd_pid=
cmd_out=

# Signals one process and the children it started. Used when the command
# could not be put in its own process group, so a group kill would hit us.
signal_tree() {
  local pid=$1 how=$2 child kids
  kids=$(ps -ax -o pid=,ppid= | awk -v p="$pid" '$2 == p { print $1 }')
  for child in $kids; do
    signal_tree "$child" "$how"
  done
  if [[ $how == KILL ]]; then
    kill -KILL "$pid" 2>/dev/null || true
  else
    kill -TERM "$pid" 2>/dev/null || true
  fi
}

# Stops the command this script is waiting on, and its children, and waits
# until that command is gone. A command that already exited leaves cmd_pid
# empty, so this does nothing and the caller still closes and removes the folder.
stop_cmd() {
  local pid=${cmd_pid:-}
  [[ -n $pid ]] || return 0
  cmd_pid=
  if ! kill -0 "$pid" 2>/dev/null; then
    wait "$pid" 2>/dev/null || true
    return 0
  fi
  local pgid shell_pgid
  pgid=$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d '[:space:]' || true)
  shell_pgid=$(ps -o pgid= -p $$ | tr -d '[:space:]' || true)
  if [[ -n $pgid && $pgid != 0 && $pgid != "$shell_pgid" ]]; then
    kill -TERM -"$pgid" 2>/dev/null || signal_tree "$pid" TERM
  else
    signal_tree "$pid" TERM
  fi
  local n=0
  while kill -0 "$pid" 2>/dev/null && [[ $n -lt 50 ]]; do
    sleep 0.1
    n=$((n + 1))
  done
  if kill -0 "$pid" 2>/dev/null; then
    if [[ -n $pgid && $pgid != 0 && $pgid != "$shell_pgid" ]]; then
      kill -KILL -"$pgid" 2>/dev/null || signal_tree "$pid" KILL
    else
      signal_tree "$pid" KILL
    fi
  fi
  wait "$pid" 2>/dev/null || true
}

cleanup() {
  status=$?
  trap - EXIT INT TERM
  # The command still running holds nothing we need for close. Stop it first
  # so a killed script does not leave it behind, then close the session.
  stop_cmd
  if [[ -n $orbe_bin && -d $project ]]; then
    # Farewell inside the daemon is one second. The rest is starting the
    # binary again. A close that never returns must not block removing the folder.
    await_orbe 15 quiet 1 "$orbe_bin" close || true
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

# Runs "$@" and waits up to $1 seconds. $2 is capture, stream, or quiet.
# $3 is 1 when the command runs in $project. Stdout of a capture or quiet
# command is left in $cmd_out. Returns 0, the command's status, or 124 when
# the deadline passes.
#
# The command is a background job in its own process group. macOS has no
# timeout(1) and no setsid(1). Job control is what gives us the group, so a
# kill later reaches a child the command started. The loop sleeps a tenth of
# a second, which macOS sleep accepts, and SECONDS is the clock so the wait
# does not drift. A command that exits is reaped on the next check.
await_orbe() {
  local limit=$1 mode=$2 in_project=$3
  shift 3
  local outf=$work/cmd.out
  : >"$outf"
  set -m
  if [[ $in_project -eq 1 ]]; then
    if [[ $mode == quiet ]]; then
      (cd "$project" && "$@") >"$outf" 2>&1 &
    elif [[ $mode == capture ]]; then
      (cd "$project" && "$@") >"$outf" &
    else
      (cd "$project" && "$@") &
    fi
  else
    if [[ $mode == quiet ]]; then
      "$@" >"$outf" 2>&1 &
    elif [[ $mode == capture ]]; then
      "$@" >"$outf" &
    else
      "$@" &
    fi
  fi
  cmd_pid=$!
  set +m
  local start=$SECONDS
  while kill -0 "$cmd_pid" 2>/dev/null; do
    if [[ $((SECONDS - start)) -ge $limit ]]; then
      stop_cmd
      if [[ $mode != stream ]]; then
        cmd_out=$(cat "$outf" 2>/dev/null || true)
      fi
      return 124
    fi
    sleep 0.1
  done
  local status=0
  wait "$cmd_pid" || status=$?
  cmd_pid=
  if [[ $mode != stream ]]; then
    cmd_out=$(cat "$outf" 2>/dev/null || true)
  fi
  return "$status"
}

# Prints the command, keeps its stdout, and stops when it exits non-zero
# or says nothing for $1 seconds. The text is left in $cmd_out. This runs
# in the current shell so the cleanup trap can see the command's pid.
finish_orbe() {
  local limit=$1
  local in_project=$2
  shift 2
  local status=0
  await_orbe "$limit" capture "$in_project" "$@" || status=$?
  if [[ $status -eq 124 ]]; then
    if [[ -n $cmd_out ]]; then
      printf '%s\n' "$cmd_out" >&2
    fi
    echo "scripts/check-release.sh: failed: $* (no answer after ${limit} s)" >&2
    exit 1
  fi
  if [[ $status -ne 0 ]]; then
    printf '%s\n' "$cmd_out" >&2
    echo "scripts/check-release.sh: failed: $* (exit $status)" >&2
    exit 1
  fi
}

capture() {
  local limit=$1
  shift
  echo "scripts/check-release.sh: $*" >&2
  finish_orbe "$limit" 0 "$@"
}

capture_in() {
  local limit=$1
  shift
  echo "scripts/check-release.sh: $*  [$project]" >&2
  finish_orbe "$limit" 1 "$@"
}

run_in() {
  local limit=$1
  shift
  echo "scripts/check-release.sh: $*  [$project]" >&2
  local status=0
  await_orbe "$limit" stream 1 "$@" || status=$?
  if [[ $status -eq 124 ]]; then
    echo "scripts/check-release.sh: failed: $* (no answer after ${limit} s)" >&2
    exit 1
  fi
  if [[ $status -ne 0 ]]; then
    echo "scripts/check-release.sh: failed: $*" >&2
    exit 1
  fi
}

# Runs `sh scripts/install.sh --tarball` on $1 into $2. The installer's own
# temp folder is under $work, so cleanup removes it even when the deadline
# kills the script. Stdout and stderr are left in $cmd_out. Returns 0, the
# installer's status, or 124 when the deadline passes.
run_install() {
  local archive=$1 dest=$2
  local status=0
  mkdir -p "$work/install-tmp"
  echo "scripts/check-release.sh: sh scripts/install.sh --tarball $(basename "$archive") --dir $dest --version $want_version" >&2
  # TMPDIR is only for this command. A later orbe command keeps the caller's temp folder.
  await_orbe 60 quiet 0 \
    env TMPDIR="$work/install-tmp" \
    sh "$scripts_dir/install.sh" \
    --tarball "$archive" --dir "$dest" --version "$want_version" || status=$?
  return "$status"
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

# The status line from a WebSocket upgrade, or nothing if the socket never
# answered. sessionUrl in the viewer builds ws://<host>/session?t=<token>
# from the fragment. Curl speaks HTTP/1.1 to that path, with the upgrade
# headers the server requires. A 101 leaves the socket open, so --max-time
# ends the wait. Two seconds is enough for a listener on this machine to
# answer, and it stops a socket that accepts and then says nothing. Curl's
# exit is not the result: timing out after a 101 still means it switched.
# The key only has to be present. The server checks the token, Host and Origin.
upgrade_status() {
  local url=$1 from=$2 hdr=$work/upgrade.hdr
  : >"$hdr"
  env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u all_proxy \
    curl --http1.1 --noproxy '*' -sS -D "$hdr" -o /dev/null \
      --max-time 2 \
      -H "Origin: $from" \
      -H 'Connection: Upgrade' \
      -H 'Upgrade: websocket' \
      -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
      -H 'Sec-WebSocket-Version: 13' \
      "$url" >"$work/upgrade.body" 2>"$work/upgrade.err" || true
  awk 'NR==1 { sub(/\r$/, ""); print; exit }' "$hdr"
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
trap 'exit 130' INT
trap 'exit 143' TERM

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

  # A copy whose checksum file does not match. install.sh must refuse it and write no binary.
  refused=$work/refused
  mkdir -p "$refused"
  if ! ln "$input" "$refused/$base" 2>/dev/null; then
    cp "$input" "$refused/$base"
  fi
  case $want in
    0*) bad_sum=1${want#0} ;;
    *) bad_sum=0${want#?} ;;
  esac
  printf '%s  %s\n' "$bad_sum" "$base" >"$refused/$base.sha256"
  status=0
  run_install "$refused/$base" "$work/refused-bin" || status=$?
  if [[ $status -eq 0 ]]; then
    printf '%s\n' "$cmd_out" >&2
    fail "install.sh installed an archive whose checksum does not match"
  fi
  if [[ $status -eq 124 ]]; then
    fail "install.sh did not refuse a wrong checksum within 60 s"
  fi
  case $cmd_out in
    *"sha256 mismatch"*) ;;
    *)
      printf '%s\n' "$cmd_out" >&2
      fail "install.sh did not refuse a wrong checksum (exit $status)"
      ;;
  esac
  [[ ! -e $work/refused-bin/orbe ]] || fail "install.sh wrote orbe for an archive whose checksum does not match"
  echo "scripts/check-release.sh: a wrong checksum was refused" >&2

  status=0
  run_install "$input" "$work/bin" || status=$?
  if [[ $status -eq 124 ]]; then
    printf '%s\n' "$cmd_out" >&2
    fail "install.sh did not finish within 60 s"
  fi
  if [[ $status -ne 0 ]]; then
    printf '%s\n' "$cmd_out" >&2
    fail "install.sh did not install the archive (exit $status)"
  fi
  printf '%s\n' "$cmd_out"
  [[ -x $work/bin/orbe ]] || fail "install.sh did not write an executable orbe"
  orbe_bin=$work/bin/orbe
else
  [[ -x $input ]] || fail "$input is not executable"
  orbe_bin=$input
fi

# Prints the version and exits. The cost is starting the binary.
capture 15 "$orbe_bin" --version
reported=$cmd_out
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

# Same startup, then one embedded file.
capture 15 "$orbe_bin" skills get core
skill=$cmd_out
[[ -n $skill ]] || fail "orbe skills get core printed nothing"
echo "scripts/check-release.sh: skills get core printed ${#skill} bytes" >&2

project=$work/project
mkdir -p "$project"

# Copies the Block's part files, including each model.
run_in 30 "$orbe_bin" add nano-button-led
# Reads the scene and every part it names.
run_in 30 "$orbe_bin" check scene.jsonc
# The same read, then the warning pass.
run_in 20 "$orbe_bin" lint scene.jsonc
# Opens a session and steps the Nano across the button press and release.
run_in 30 "$orbe_bin" test

# Starts the session daemon and waits until the scene is open.
capture_in 15 "$orbe_bin" open
opened=$cmd_out
printf '%s\n' "$opened"
case $opened in
  opened\ *) ;;
  *) fail "orbe open did not open the scene: $opened" ;;
esac
# 100 ms on the Nano. The session is already up, so this is the step itself.
run_in 20 "$orbe_bin" step 100ms
# One read from the open session.
capture_in 15 "$orbe_bin" get @led1
reading=$cmd_out
printf '%s\n' "$reading"
[[ -n $reading ]] || fail "orbe get @led1 printed nothing"
# The daemon's farewell is one second, then the client returns.
capture_in 15 "$orbe_bin" close
closed=$cmd_out
printf '%s\n' "$closed"
case $closed in
  *"closed at t="*) ;;
  *) fail "orbe close did not say the session closed: $closed" ;;
esac

# show serves the page for the session open just above, so open again after
# that close. The close after the fetch is what stops the server.
capture_in 15 "$orbe_bin" open
opened=$cmd_out
printf '%s\n' "$opened"
case $opened in
  opened\ *) ;;
  *) fail "orbe open did not open the scene: $opened" ;;
esac
# Prints the page URL once the listener is bound. It does not wait for a browser.
capture_in 15 "$orbe_bin" show --no-browser
shown=$cmd_out
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

# The fragment is #t=<token>. The viewer opens /session?t=<token> on this host.
command -v curl >/dev/null 2>&1 || fail "need curl to open the session socket"
token=${url#*#t=}
case $token in
  *0) wrong_token=${token%0}1 ;;
  *) wrong_token=${token%?}0 ;;
esac
session_url="http://${origin#http://}/session?t=${token}"
wrong_url="http://${origin#http://}/session?t=${wrong_token}"
session_status=$(upgrade_status "$session_url" "$origin")
if [[ $session_status != "HTTP/1.1 101 Switching Protocols" ]]; then
  if [[ -s $work/upgrade.err ]]; then
    cat "$work/upgrade.err" >&2
  fi
  fail "the session socket did not answer 101${session_status:+: $session_status}"
fi
echo "scripts/check-release.sh: the session answered 101" >&2
refused=$(upgrade_status "$wrong_url" "$origin")
if [[ $refused != "HTTP/1.1 403 Forbidden" ]]; then
  if [[ -s $work/upgrade.err ]]; then
    cat "$work/upgrade.err" >&2
  fi
  fail "a wrong token was not refused${refused:+: $refused}"
fi
echo "scripts/check-release.sh: a wrong token was refused" >&2

# The scene names each model as /parts/<id>/<id>.glb. nano-board is the board
# in this Block. The bytes are the glTF binary magic.
model_url="$origin/parts/nano-board/nano-board.glb"
fetch "$model_url" "$work/model.glb"
magic=$(od -An -tx1 -N 4 "$work/model.glb" | tr -d '[:space:]' | tr 'A-F' 'a-f')
[[ $magic == 676c5446 ]] || fail "$model_url does not start with the glTF magic (first bytes ${magic:-none})"
echo "scripts/check-release.sh: model loaded" >&2

capture_in 15 "$orbe_bin" close
closed=$cmd_out
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
