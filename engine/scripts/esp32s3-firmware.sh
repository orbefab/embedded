#!/usr/bin/env bash
# Rebuilds the bare-metal ESP32-S3 programs of examples/esp32s3-blink and examples/esp32s3-pattern
# from firmware/esp32s3, files each in its example's blobs folder under its SHA-256, and writes that
# hash and its length into the example's netlist. With --check it only rebuilds, compares each
# with the committed blob, and checks that the netlist names that hash and length.
#
# Needs xtensa-esp32s3-elf-gcc on PATH, or its path in XTENSA_ESP32S3_GCC. The committed blobs
# came from GCC 14.2.0, crosstool-NG esp-14.2.0_20260121, and another compiler builds other
# bytes. A build of the engine reads only the committed blobs.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
gcc=${XTENSA_ESP32S3_GCC:-xtensa-esp32s3-elf-gcc}
check=0
if [[ ${1:-} == --check ]]; then
  check=1
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$root/firmware/esp32s3"
flags=(-mabi=call0 -O2 -g0 -ffreestanding -nostdlib -nostartfiles -fno-builtin
  -Wall -Wextra -Werror -Wl,-T,ram.ld -Wl,--build-id=none -Wl,-n)
"$gcc" "${flags[@]}" -DPERIOD=4000000 -o "$work/blink.elf" start.S blink.c
"$gcc" "${flags[@]}" -o "$work/pattern.elf" pattern_start.S pattern.c

# file_blob ELF EXAMPLE: checks or files ELF as the firmware of examples/EXAMPLE.
file_blob() {
  local elf=$1 example=$root/examples/$2 sha bytes
  sha=$(sha256sum "$elf" | cut -d' ' -f1)
  bytes=$(wc -c <"$elf")
  if ((check)); then
    cmp "$elf" "$example/blobs/$sha"
    python3 -I -c '
import json, sys
path, sha, size = sys.argv[1], sys.argv[2], int(sys.argv[3])
named = [part["firmware"]["elf"] for part in json.load(open(path))["parts"] if "firmware" in part]
if named != [{"sha256": sha, "bytes": size}]:
    sys.exit(f"{path} names {named}, not sha256 {sha} and bytes {size}")
' "$example/netlist.json" "$sha" "$bytes"
    echo "$(basename "$elf") rebuilds byte for byte: $sha $bytes"
    return
  fi
  mkdir -p "$example/blobs"
  rm -f "$example"/blobs/*
  cp "$elf" "$example/blobs/$sha"
  sed -i -E "s/(\"sha256\": \")[0-9a-f]{64}/\1$sha/; s/(\"bytes\": )[0-9]+/\1$bytes/" "$example/netlist.json"
  echo "$2: $sha $bytes"
}

file_blob "$work/blink.elf" esp32s3-blink
file_blob "$work/pattern.elf" esp32s3-pattern
