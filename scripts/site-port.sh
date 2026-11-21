#!/usr/bin/env bash
# A free TCP port, and a retry when the server we start finds that port taken.
# free_port closes its socket before the caller binds, so another process can
# take the port in between. with_fresh_ports runs the command again with new
# ports when the failure is that race and not a test failure.

free_port() {
  node -e 'const s = require("node:net").createServer().listen(0, "127.0.0.1", () => { process.stdout.write(String(s.address().port) + "\n"); s.close(); })'
}

# 0 when the text is the port race and not a Playwright failure line.
port_was_taken() {
  local text=$1
  if [[ $text =~ [0-9]+[[:space:]]failed ]]; then
    return 1
  fi
  [[ $text == *"is already in use"* || $text == *"is already used"* || $text == *"EADDRINUSE"* || $text == *"Address already in use"* ]]
}

# with_fresh_ports VAR [VAR...] -- command...
# Exports each VAR to a free port, runs the command, and retries up to 4 more
# times when the command fails only because a port was taken.
with_fresh_ports() {
  local -a vars=()
  while [[ $# -gt 0 && $1 != -- ]]; do
    vars+=("$1")
    shift
  done
  if [[ ${1:-} != -- ]]; then
    echo "with_fresh_ports: expected -- before the command" >&2
    return 2
  fi
  shift
  local log status=1 tries=5
  log=$(mktemp)
  while [[ $tries -gt 0 ]]; do
    tries=$((tries - 1))
    local shown=""
    local v value
    for v in "${vars[@]}"; do
      value=$(free_port)
      printf -v "$v" '%s' "$value"
      export "${v?}"
      shown+="$v=$value "
    done
    echo "check-site: $shown"
    # pipefail makes the pipeline report the command, and `||` keeps a failure
    # from exiting under `set -e` before this function can retry or return it.
    status=0
    set -o pipefail
    "$@" 2>&1 | tee "$log" || status=$?
    if [[ $status -eq 0 ]]; then
      rm -f "$log"
      return 0
    fi
    if [[ $tries -gt 0 ]] && port_was_taken "$(cat "$log")"; then
      echo "check-site: a port was taken, trying another" >&2
      continue
    fi
    break
  done
  rm -f "$log"
  return "$status"
}
