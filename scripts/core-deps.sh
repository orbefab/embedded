#!/usr/bin/env bash
# Fails when a packages/cores crate depends on a packages/socs crate.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v python3 >/dev/null 2>&1; then
  echo "scripts/core-deps.sh: python3 is not installed. Install it (apt install python3) and re-run." >&2
  exit 1
fi

exec python3 - "$@" <<'PY'
import json
import subprocess
import sys
from pathlib import Path

meta = json.loads(
    subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"]
    )
)
root = Path(meta["workspace_root"])
failed = False
for pkg in meta["packages"]:
    manifest = Path(pkg["manifest_path"])
    try:
        rel = manifest.relative_to(root).as_posix()
    except ValueError:
        continue
    if not rel.startswith("packages/cores/"):
        continue
    for dep in pkg["dependencies"]:
        path = dep.get("path")
        if not path:
            continue
        try:
            dep_rel = Path(path).relative_to(root).as_posix()
        except ValueError:
            continue
        if dep_rel.startswith("packages/socs/"):
            kind = dep.get("kind") or "normal"
            print(
                f"scripts/core-deps.sh: {pkg['name']} ({kind}) depends on {dep['name']} at {dep_rel}",
                file=sys.stderr,
            )
            failed = True
if failed:
    sys.exit(1)
PY
