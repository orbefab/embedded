#!/usr/bin/env bash
# Builds the console sketch, files its app and partition table in the blob cache, and fails
# unless those hashes are the ones the verifier netlist names. The ELF hash is pinned below.
# With --check it then runs that netlist and scripts/esp32s3_arduino_console.py --check.
#
#   scripts/esp32s3-arduino.sh console
#   scripts/esp32s3-arduino.sh --check console
#
# It needs Arduino CLI 1.5.1 and the installed esp32:esp32 core 3.3.12. It installs nothing
# and downloads nothing. The compile's working directory is the temporary folder, which the
# prefix map rewrites, so the image does not record the directory the script was started from.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
check=0
if [[ ${1:-} == --check ]]; then
  check=1
  shift
fi
if [[ $# -lt 1 ]]; then
  echo "usage: scripts/esp32s3-arduino.sh [--check] console" >&2
  exit 2
fi
name=$1
if [[ $name != console ]]; then
  echo "esp32s3-arduino.sh: unknown sketch $name" >&2
  exit 2
fi

sdkconfig=$HOME/.arduino15/packages/esp32/tools/esp32s3-libs/3.3.12/qio_qspi/include/sdkconfig.h
sdkconfig_sha=f44c8812a67ab0c3c25704efd4aac3be2389119c65fc6179349aa7475a8405fa
elf_sha=143e2e2f6b5f77c152b48c5b2b516d66c9d7ce09a9671821a78dd0c1818be0b7
version=$(arduino-cli version)
if [[ $version != "arduino-cli  Version: 1.5.1 "* ]]; then
  echo "esp32s3-arduino.sh: arduino-cli is not 1.5.1: $version" >&2
  exit 1
fi
if ! arduino-cli core list | grep -q '^esp32:esp32  *3\.3\.12 '; then
  echo "esp32s3-arduino.sh: esp32:esp32 3.3.12 is not installed" >&2
  exit 1
fi
got_sdk=$(sha256sum "$sdkconfig" | cut -d' ' -f1)
if [[ $got_sdk != "$sdkconfig_sha" ]]; then
  echo "esp32s3-arduino.sh: sdkconfig.h is $got_sdk, not $sdkconfig_sha" >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sketch=$work/console
mkdir -p "$sketch"
cp "$root/firmware/esp32s3/arduino/console/console.ino" "$sketch/"
build=$work/build
out=$work/out
fqbn=esp32:esp32:esp32s3:CDCOnBoot=cdc,USBMode=hwcdc,FlashSize=16M,PSRAM=disabled
map="-ffile-prefix-map=$work=/src -ffile-prefix-map=$build=/build -ffile-prefix-map=$HOME/.arduino15=/arduino15"
(
  cd "$work"
  SOURCE_DATE_EPOCH=1767225600 arduino-cli compile --clean --fqbn "$fqbn" --jobs 2 \
    --build-path "$build" --output-dir "$out" \
    --build-property "compiler.c.extra_flags=$map" \
    --build-property "compiler.cpp.extra_flags=$map" \
    --build-property "compiler.S.extra_flags=$map" \
    "$sketch"
)

sha() {
  sha256sum "$1" | cut -d' ' -f1
}
bin=$out/console.ino.bin
elf=$out/console.ino.elf
table=$out/console.ino.partitions.bin
bin_sha=$(sha "$bin")
table_sha=$(sha "$table")
got_elf=$(sha "$elf")
echo "console.ino.bin: $bin_sha $(wc -c < "$bin")"
echo "console.ino.elf: $got_elf $(wc -c < "$elf")"
echo "console.ino.partitions.bin: $table_sha $(wc -c < "$table")"
if [[ $got_elf != "$elf_sha" ]]; then
  echo "esp32s3-arduino.sh: the ELF is $got_elf, not $elf_sha" >&2
  exit 1
fi

netlist=$root/firmware/esp32s3/arduino/checks/console/netlist.json
python3 -I - "$netlist" "$table_sha" "$bin_sha" <<'PY'
import json, sys
path, table, app = sys.argv[1:]
part = next(part for part in json.load(open(path))["parts"] if part["kind"] == "esp32s3")
regions = part["firmware"]["flash"]
named = [(region["offset"], region["sha256"]) for region in regions]
want = [(32768, table), (65536, app)]
if named != want:
    sys.exit(f"{path} names {named}, not {want}")
PY

cache=$( "$root/scripts/blobs.sh" | tail -n 1)
cp "$table" "$cache/$table_sha"
cp "$bin" "$cache/$bin_sha"
if [[ $(sha "$cache/$table_sha") != "$table_sha" || $(sha "$cache/$bin_sha") != "$bin_sha" ]]; then
  echo "esp32s3-arduino.sh: the blob cache does not hold the built images" >&2
  exit 1
fi

if ((check)); then
  run=${CARGO_TARGET_DIR:-$root/target}/esp32s3-arduino-console-check
  rm -rf "$run"
  (
    cd "$root"
    echo "rustc $(rustc --version)"
    cargo run -q -p sim-cli --locked -- run "$netlist" --out "$run" --blobs "$cache"
  )
  uv run "$root/scripts/esp32s3_arduino_console.py" --check "$run"
  rm -rf "$run"
fi
