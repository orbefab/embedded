#!/usr/bin/env bash
# Shared turn-taking lock and default job limits for heavy local check scripts.
# Sourced by scripts/check.sh, scripts/run-hashes.sh and scripts/release-build.sh.
# Uses flock(1) on one per-user file in /tmp so every worktree shares one queue,
# keeping holder details in a small file beside the lock. Outside CI, defaults
# CARGO_BUILD_JOBS to 2 and NEXTEST_TEST_THREADS to 4 when unset. Skips locking
# in CI, on nested calls, or where flock is not installed.

# Call from the repo root after validating arguments:
#   check_lock_exec <script> <label> [-- <arg>...]
check_lock_exec() {
  local script=$1 label=$2
  shift 2
  if [[ ${1:-} == -- ]]; then
    shift
  fi

  if [[ -z ${CI:-} ]]; then
    export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
    export NEXTEST_TEST_THREADS=${NEXTEST_TEST_THREADS:-4}
  else
    return 0
  fi

  local uid lock_file info_file tmp holder
  uid=${UID:-$(id -u)}
  lock_file="/tmp/orbe-check-${uid}.lock"
  info_file="/tmp/orbe-check-${uid}.holder"

  if [[ -n ${ORBE_CHECK_LOCK_HELD:-} ]]; then
    if [[ -n ${ORBE_CHECK_LOCK_WRITE_HOLDER:-} ]]; then
      tmp="${info_file}.$$"
      printf '%s (%s) since %s\n' "$PWD" "$label" "$(date '+%Y-%m-%d %H:%M:%S %Z')" >"$tmp"
      mv -f "$tmp" "$info_file"
      unset ORBE_CHECK_LOCK_WRITE_HOLDER
    fi
    return 0
  fi

  if ! command -v flock >/dev/null 2>&1; then
    echo "${script}: flock is not installed; running without the check lock" >&2
    export ORBE_CHECK_LOCK_HELD=1
    return 0
  fi

  if ! flock -n "$lock_file" true 2>/dev/null; then
    holder=$(cat "$info_file" 2>/dev/null || true)
    if [[ -n $holder ]]; then
      echo "${script}: waiting for check lock held by ${holder}" >&2
    else
      echo "${script}: waiting for check lock" >&2
    fi
  fi

  ORBE_CHECK_LOCK_HELD=1 ORBE_CHECK_LOCK_WRITE_HOLDER=1 \
    exec flock -o "$lock_file" bash "$script" "$@"
}
