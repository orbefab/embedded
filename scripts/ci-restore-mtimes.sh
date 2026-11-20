#!/usr/bin/env bash
# Gives each tracked file the mtime of the commit that last changed it.
# cargo treats a file newer than the cached build as dirty. A checkout sets
# every mtime to now, so a restored workspace target rebuilds anyway. This
# needs the commit that last touched each file, which means a full checkout.
# Usage: scripts/ci-restore-mtimes.sh
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $# -gt 0 ]]; then
  echo "usage: scripts/ci-restore-mtimes.sh" >&2
  exit 2
fi

python3 - <<'PY'
import os
import subprocess

log = subprocess.run(
    ["git", "log", "--pretty=format:%ct", "--name-only", "--diff-filter=AMT"],
    check=True,
    capture_output=True,
    text=True,
)
seen = set()
stamp = None
for line in log.stdout.splitlines():
    if line == "":
        continue
    if line.isdigit() and len(line) >= 9:
        stamp = int(line)
        continue
    if stamp is None or line in seen:
        continue
    seen.add(line)
    if os.path.lexists(line):
        os.utime(line, (stamp, stamp), follow_symlinks=False)
PY
