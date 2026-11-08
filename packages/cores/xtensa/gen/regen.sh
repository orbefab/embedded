#!/usr/bin/env bash
# Regenerates src/decode/{op,esp32,esp32s3}.rs from the two xtensa-modules.c files.
#
# Usage: packages/cores/xtensa/gen/regen.sh OVERLAYS SCRATCH_DIR
#
# OVERLAYS is a checkout of the xtensa-overlays repository at the commit below. generate.py checks
# each module's SHA-256 and checks its C expressions against gcc, so gcc must be on PATH. The
# generator writes into SCRATCH_DIR, and this script copies the three files into the crate.
set -euo pipefail
if [[ $# -ne 2 ]]; then
  echo "usage: $0 OVERLAYS SCRATCH_DIR" >&2
  exit 2
fi
overlays=$1
out=$2
here=$(cd "$(dirname "$0")" && pwd)
python3 "$here/generate.py" --config both \
  --commit dd1cf19f6eb327a9db51043439974a6de13f5c7f \
  --esp32 "$overlays/xtensa_esp32/binutils/bfd/xtensa-modules.c" \
  --esp32-sha bf5414d741d7250b3725d24c43220b147eefb140cfdaf3c38e46dbc7a538bc32 \
  --esp32s3 "$overlays/xtensa_esp32s3/binutils/bfd/xtensa-modules.c" \
  --esp32s3-sha a54135d8e85996f7aac0e31b10b021ee26d4db1b6cf893eef081560a816c4f7a \
  -o "$out"
for name in op esp32 esp32s3; do
  cp "$out/$name.rs" "$here/../src/decode/$name.rs"
done
