#!/usr/bin/env bash
# Fails when a crate depends on one its layer must not reach: a packages/cores
# crate on a packages/socs crate, packages/mcu on anything but packages/model,
# or a packages/socs crate on packages/engine.
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

# (crate folder prefix, why a path dependency is refused)
RULES = [
    ("packages/cores/", lambda dep: dep.startswith("packages/socs/")),
    ("packages/mcu", lambda dep: dep != "packages/model"),
    ("packages/socs/", lambda dep: dep.startswith("packages/engine")),
]

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
        rel = manifest.parent.relative_to(root).as_posix()
    except ValueError:
        continue
    refused = [why for prefix, why in RULES if rel.startswith(prefix)]
    if not refused:
        continue
    for dep in pkg["dependencies"]:
        path = dep.get("path")
        if not path:
            continue
        try:
            dep_rel = Path(path).relative_to(root).as_posix()
        except ValueError:
            continue
        if any(why(dep_rel) for why in refused):
            kind = dep.get("kind") or "normal"
            print(
                f"scripts/core-deps.sh: {pkg['name']} ({kind}) depends on {dep['name']} at {dep_rel}",
                file=sys.stderr,
            )
            failed = True
if failed:
    sys.exit(1)
PY
