#!/usr/bin/env bash
# Rebuilds the ESP32-S3 firmware of the examples, files each image in its example's blobs folder
# under its SHA-256, and writes that hash and its length into the example's netlist. With --check
# it only rebuilds, compares each image with the committed blob, and checks that the netlist names
# that hash and length.
#
#   packages/engine/scripts/esp32s3-firmware.sh [--check] [bare-metal] [idf]
#
# bare-metal builds the programs of packages/engine/tests/circuits/esp32s3-blink and packages/engine/tests/circuits/esp32s3-pattern from
# packages/engine/tests/firmware/esp32s3. It needs xtensa-esp32s3-elf-gcc on PATH, or its path in XTENSA_ESP32S3_GCC.
# Those blobs came from GCC 14.2.0, crosstool-NG esp-14.2.0_20260121.
#
# idf builds the hello_world app and partition table of packages/engine/tests/circuits/esp32s3-hello, the screen app
# and partition table of packages/engine/tests/circuits/esp32s3-screen, the encoder app and partition table of
# packages/engine/tests/circuits/esp32s3-encoder, the touch app and partition table of packages/engine/tests/circuits/esp32s3-touch,
# the dualcore app and partition table of packages/engine/tests/circuits/esp32s3-dualcore,
# the echo app and partition table of packages/engine/tests/circuits/esp32s3-echo,
# the ring app and partition table of packages/engine/tests/circuits/esp32s3-ring,
# and the uart app and partition table of packages/engine/tests/circuits/esp32s3-uart,
# from packages/engine/tests/firmware/esp32s3/idf with ESP-IDF v5.5.5. It needs that IDF's
# export.sh sourced, so that IDF_PATH is set and idf.py and its toolchain are on PATH. The
# screen app also needs the components its dependencies.lock pins, which IDF's component manager
# fetches or takes from IDF_COMPONENT_CACHE_PATH, and checks against the pinned hashes. The
# builds are reproducible, so they do not depend on where the tree or the build lies.
#
# With neither named, it builds both. Another compiler or IDF builds other bytes, and a build of
# the engine reads only the committed blobs.
set -euo pipefail
root=$(cd "$(dirname "$0")/../../.." && pwd)
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

# file_blobs EXAMPLE FIRMWARE FILE...: checks or files FILE... as the blobs of packages/engine/tests/circuits/EXAMPLE, in
# the order its netlist's firmware names them. FIRMWARE is `elf` or `flash`.
file_blobs() {
  local example=$root/packages/engine/tests/circuits/$1 kind=$2 file
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
    -Wall -Wextra -Werror "-Wl,-T,ram.ld" "-Wl,--build-id=none" "-Wl,-n")
  (
    cd "$root/packages/engine/tests/firmware/esp32s3"
    "$gcc" "${flags[@]}" -DPERIOD=4000000 -o "$work/blink.elf" start.S blink.c
    "$gcc" "${flags[@]}" -o "$work/pattern.elf" pattern_start.S pattern.c
  )
  file_blobs esp32s3-blink elf "$work/blink.elf"
  file_blobs esp32s3-pattern elf "$work/pattern.elf"
}

# idf_build APP: builds packages/engine/tests/firmware/esp32s3/idf/APP in the copy of the tree under $work/idf.
idf_build() {
  local project=$work/idf/$1
  idf.py --project-dir "$project" --build-dir "$project/build" set-target esp32s3 >"$work/$1-set-target.log" 2>&1 ||
    { cat "$work/$1-set-target.log" >&2; return 1; }
  idf.py --project-dir "$project" --build-dir "$project/build" build >"$work/$1-build.log" 2>&1 ||
    { cat "$work/$1-build.log" >&2; return 1; }
}

idf() {
  : "${IDF_PATH:?source ESP-IDF v5.5.5 export.sh first}"
  local hello=$work/idf/hello_world/build screen=$work/idf/screen/build
  local encoder=$work/idf/encoder/build touch=$work/idf/touch/build
  local dualcore=$work/idf/dualcore/build
  local echo=$work/idf/echo/build
  local ring=$work/idf/ring/build
  local uart=$work/idf/uart/build
  cp -r "$root/packages/engine/tests/firmware/esp32s3/idf" "$work/idf"
  idf_build hello_world
  idf_build screen
  idf_build encoder
  idf_build touch
  idf_build dualcore
  idf_build echo
  idf_build ring
  idf_build uart
  file_blobs esp32s3-hello flash "$hello/partition_table/partition-table.bin" "$hello/hello_world.bin"
  file_blobs esp32s3-screen flash "$screen/partition_table/partition-table.bin" "$screen/screen.bin"
  file_blobs esp32s3-encoder flash "$encoder/partition_table/partition-table.bin" "$encoder/encoder.bin"
  file_blobs esp32s3-touch flash "$touch/partition_table/partition-table.bin" "$touch/touch.bin"
  file_blobs esp32s3-dualcore flash "$dualcore/partition_table/partition-table.bin" "$dualcore/dualcore.bin"
  file_blobs esp32s3-echo flash "$echo/partition_table/partition-table.bin" "$echo/echo.bin"
  file_blobs esp32s3-ring flash "$ring/partition_table/partition-table.bin" "$ring/ring.bin"
  file_blobs esp32s3-uart flash "$uart/partition_table/partition-table.bin" "$uart/uart.bin"
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
