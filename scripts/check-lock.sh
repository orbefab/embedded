#!/usr/bin/env bash
# Shared turn-taking lock and default job limits for heavy local check scripts.
# Sourced by scripts/check.sh, scripts/run-hashes.sh and scripts/release-build.sh.
# Holds an flock(1) lock on FD 9 of one per-user file in /tmp so every worktree
# shares one queue and child processes inherit the lock until they all exit,
# keeping holder details in a small file beside the lock. Outside CI, defaults
# CARGO_BUILD_JOBS to 2 and NEXTEST_TEST_THREADS to 4 when unset. Skips locking
# in CI, on nested calls, or where flock is not installed.

# Call from the repo root after validating arguments:
#   check_lock <script> <label>
check_lock() {
  local script=$1 label=$2

  if [[ -z ${CI:-} ]]; then
    export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
    export NEXTEST_TEST_THREADS=${NEXTEST_TEST_THREADS:-4}
  else
    return 0
  fi

  if [[ -n ${ORBE_CHECK_LOCK_HELD:-} ]]; then
    return 0
  fi

  if ! command -v flock >/dev/null 2>&1; then
    echo "${script}: flock is not installed; running without the check lock" >&2
    export ORBE_CHECK_LOCK_HELD=1
    return 0
  fi

  local uid lock_file info_file tmp holder
  uid=${UID:-$(id -u)}
  lock_file="/tmp/orbe-check-${uid}.lock"
  info_file="/tmp/orbe-check-${uid}.holder"

  if ! { exec 9>>"$lock_file"; } 2>/dev/null; then
    echo "${script}: cannot open check lock ${lock_file}; remove it and re-run" >&2
    exit 1
  fi

  if ! flock -n 9; then
    holder=$(cat "$info_file" 2>/dev/null || true)
    if [[ -n $holder ]]; then
      echo "${script}: waiting for check lock held by ${holder}" >&2
    else
      echo "${script}: waiting for check lock" >&2
    fi
    flock 9
  fi

  tmp="${info_file}.$$"
  printf '%s (%s) since %s\n' "$PWD" "$label" "$(date '+%Y-%m-%d %H:%M:%S %Z')" >"$tmp"
  mv -f "$tmp" "$info_file"
  export ORBE_CHECK_LOCK_HELD=1
}
