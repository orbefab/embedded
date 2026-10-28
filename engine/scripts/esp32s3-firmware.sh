#!/usr/bin/env bash
# Rebuilds the bare-metal ESP32-S3 program of examples/esp32s3-blink from firmware/esp32s3,
# files it in the example's blobs folder under its SHA-256, and writes that hash and its length
# into the example's netlist. With --check it only rebuilds, compares with the committed blob, and
# checks that the netlist names that hash and length.
#
# Needs xtensa-esp32s3-elf-gcc on PATH, or its path in XTENSA_ESP32S3_GCC. The committed blob
# came from GCC 14.2.0, crosstool-NG esp-14.2.0_20260121, and another compiler builds other
# bytes. A build of the engine reads only the committed blob.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
gcc=${XTENSA_ESP32S3_GCC:-xtensa-esp32s3-elf-gcc}
example=$root/examples/esp32s3-blink
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$root/firmware/esp32s3"
"$gcc" -mabi=call0 -O2 -g0 -ffreestanding -nostdlib -nostartfiles -fno-builtin \
  -DPERIOD=4000000 -Wall -Wextra -Werror \
  -Wl,-T,blink.ld -Wl,--build-id=none -Wl,-n \
  -o "$work/blink.elf" start.S blink.c
sha=$(sha256sum "$work/blink.elf" | cut -d' ' -f1)
bytes=$(wc -c <"$work/blink.elf")
if [[ ${1:-} == --check ]]; then
  cmp "$work/blink.elf" "$example/blobs/$sha"
  python3 -I -c '
import json, sys
path, sha, size = sys.argv[1], sys.argv[2], int(sys.argv[3])
named = [part["firmware"]["elf"] for part in json.load(open(path))["parts"] if "firmware" in part]
if named != [{"sha256": sha, "bytes": size}]:
    sys.exit(f"examples/esp32s3-blink/netlist.json names {named}, not sha256 {sha} and bytes {size}")
' "$example/netlist.json" "$sha" "$bytes"
  echo "blink.elf rebuilds byte for byte: $sha $bytes"
  exit 0
fi
mkdir -p "$example/blobs"
rm -f "$example"/blobs/*
cp "$work/blink.elf" "$example/blobs/$sha"
sed -i -E "s/(\"sha256\": \")[0-9a-f]{64}/\1$sha/; s/(\"bytes\": )[0-9]+/\1$bytes/" "$example/netlist.json"
echo "$sha $bytes"
