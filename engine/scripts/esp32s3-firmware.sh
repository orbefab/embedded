#!/usr/bin/env bash
# Rebuilds the ESP32-S3 firmware of the examples, files each image in its example's blobs folder
# under its SHA-256, and writes that hash and its length into the example's netlist. With --check
# it only rebuilds, compares each image with the committed blob, and checks that the netlist names
# that hash and length.
#
#   scripts/esp32s3-firmware.sh [--check] [bare-metal] [idf]
#
# bare-metal builds the programs of examples/esp32s3-blink and examples/esp32s3-pattern from
# firmware/esp32s3. It needs xtensa-esp32s3-elf-gcc on PATH, or its path in XTENSA_ESP32S3_GCC.
# Those blobs came from GCC 14.2.0, crosstool-NG esp-14.2.0_20260121.
#
# idf builds the hello_world app and partition table of examples/esp32s3-hello from
# firmware/esp32s3/idf with ESP-IDF v5.5.5. It needs that IDF's export.sh sourced, so that
# IDF_PATH is set and idf.py and its toolchain are on PATH. The build is reproducible, so it does
# not depend on where the tree or the build lies.
#
# With neither named, it builds both. Another compiler or IDF builds other bytes, and a build of
# the engine reads only the committed blobs.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
check=0
if [[ ${1:-} == --check ]]; then
  check=1
  shift
fi
groups=("$@")
if ((${#groups[@]} == 0)); then
  groups=(bare-metal idf)
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# file_blobs EXAMPLE FIRMWARE FILE...: checks or files FILE... as the blobs of examples/EXAMPLE, in
# the order its netlist's firmware names them. FIRMWARE is `elf` or `flash`.
file_blobs() {
  local example=$root/examples/$1 kind=$2 file
  shift 2
  if ((check)); then
    for file in "$@"; do
      cmp "$file" "$example/blobs/$(sha256sum "$file" | cut -d' ' -f1)"
    done
  else
    mkdir -p "$example/blobs"
    find "$example/blobs" -type f -delete
    for file in "$@"; do
      cp "$file" "$example/blobs/$(sha256sum "$file" | cut -d' ' -f1)"
    done
  fi
  python3 -I - "$example/netlist.json" "$kind" "$check" "$@" <<'EOF'
import hashlib, json, re, sys
path, kind, check, files = sys.argv[1], sys.argv[2], sys.argv[3] == "1", sys.argv[4:]
named = [blob for blob in (
    part["firmware"][kind] for part in json.load(open(path))["parts"] if "firmware" in part
) for blob in (blob if isinstance(blob, list) else [blob])]
built = []
for file in files:
    data = open(file, "rb").read()
    built.append({"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)})
if len(named) != len(built):
    sys.exit(f"{path} names {len(named)} blobs as its {kind} firmware, not {len(built)}")
for blob, new in zip(named, built):
    old = {"sha256": blob["sha256"], "bytes": blob["bytes"]}
    if old == new:
        continue
    if check:
        sys.exit(f"{path} names {old}, not {new}")
    text = open(path).read()
    pattern = f'"sha256": "{old["sha256"]}",\n(\\s*)"bytes": {old["bytes"]}'
    text, count = re.subn(pattern, f'"sha256": "{new["sha256"]}",\n\\1"bytes": {new["bytes"]}', text)
    if count != 1:
        sys.exit(f"{path} names {old} {count} times")
    open(path, "w").write(text)
for file, new in zip(files, built):
    print(f"{file.rsplit('/', 1)[-1]}: {new['sha256']} {new['bytes']}")
EOF
}

bare_metal() {
  local gcc=${XTENSA_ESP32S3_GCC:-xtensa-esp32s3-elf-gcc}
  local flags=(-mabi=call0 -O2 -g0 -ffreestanding -nostdlib -nostartfiles -fno-builtin
    -Wall -Wextra -Werror -Wl,-T,ram.ld -Wl,--build-id=none -Wl,-n)
  (
    cd "$root/firmware/esp32s3"
    "$gcc" "${flags[@]}" -DPERIOD=4000000 -o "$work/blink.elf" start.S blink.c
    "$gcc" "${flags[@]}" -o "$work/pattern.elf" pattern_start.S pattern.c
  )
  file_blobs esp32s3-blink elf "$work/blink.elf"
  file_blobs esp32s3-pattern elf "$work/pattern.elf"
}

idf() {
  : "${IDF_PATH:?source ESP-IDF v5.5.5 export.sh first}"
  local tree=$work/idf project=$work/idf/hello_world
  cp -r "$root/firmware/esp32s3/idf" "$tree"
  idf.py --project-dir "$project" --build-dir "$project/build" set-target esp32s3 >"$work/set-target.log" 2>&1 ||
    { cat "$work/set-target.log" >&2; return 1; }
  idf.py --project-dir "$project" --build-dir "$project/build" build >"$work/build.log" 2>&1 ||
    { cat "$work/build.log" >&2; return 1; }
  file_blobs esp32s3-hello flash "$project/build/partition_table/partition-table.bin" \
    "$project/build/hello_world.bin"
}

for group in "${groups[@]}"; do
  case $group in
    bare-metal) bare_metal ;;
    idf) idf ;;
    *)
      echo "esp32s3-firmware.sh: unknown group $group" >&2
      exit 2
      ;;
  esac
done
if ((check)); then
  echo "every image rebuilds byte for byte"
fi
