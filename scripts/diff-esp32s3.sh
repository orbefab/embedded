#!/usr/bin/env bash
# Differential check: boots stock ESP32-S3 apps in both Espressif's QEMU and
# orbe and fails with a unified diff if their normalized serial output differs.
# Run by hand before a release; not part of PR CI or nightly.
#
# Usage: scripts/diff-esp32s3.sh [--app NAME ...] [--list] [--cache DIR] [--orbe-run PATH]
#
# Builds `orbe-run` when ORBE_RUN is unset and no --orbe-run flag is given.
# Fetches the pinned Espressif QEMU release and Arduino-ESP32 3.3.12 bootloaders
# into ${ORBE_DIFF_SIM_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/orbe-diff-sim}
# when missing (verifying pinned SHA-256 digests) and builds user-space stubs
# for libSDL2/libslirp with cc if needed. Installs no system packages.
# Outside CI, scripts/check-lock.sh takes the shared per-user check lock.
set -euo pipefail
cd "$(dirname "$0")/.."

need_build=1
for arg in "$@"; do
  case "$arg" in
    --list | -h | --help | --orbe-run | --orbe-run=*) need_build=0 ;;
  esac
done
if [[ -n ${ORBE_RUN:-} ]]; then
  need_build=0
fi

if ((need_build)); then
  # shellcheck source=scripts/check-lock.sh
  source scripts/check-lock.sh
  check_lock "scripts/diff-esp32s3.sh" "scripts/diff-esp32s3.sh $*"
  cargo build -q -p orbe-engine --features runner --bin orbe-run --locked
fi

exec python3 scripts/diff_esp32s3.py "$@"
