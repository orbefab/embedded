#!/usr/bin/env bash
# Builds one Arduino sketch, files its app and partition table in the blob cache, and fails
# unless those hashes are the ones the verifier netlist names. The ELF hashes are pinned below.
# With --check, the sketch is built, its netlist is run with --blobs set to the cache,
# and the matching checker runs on the folder.
#
#   packages/engine/scripts/esp32s3-arduino.sh console
#   packages/engine/scripts/esp32s3-arduino.sh --check console
#   packages/engine/scripts/esp32s3-arduino.sh knob LVGL_DIR
#   packages/engine/scripts/esp32s3-arduino.sh --check knob LVGL_DIR
#
# knob stages LVGL_DIR with the knob panel library and files the ELF in the blob cache.
# It needs Arduino CLI 1.5.1 and the installed esp32:esp32 core 3.3.12. It installs nothing
# and downloads nothing. The compile's working directory is the temporary folder, which the
# prefix map rewrites, so the image does not record the directory the script was started from.
set -euo pipefail
root=$(cd "$(dirname "$0")/../../.." && pwd)
check=0
if [[ ${1:-} == --check ]]; then
  check=1
  shift
fi
if [[ $# -lt 1 ]]; then
  echo "usage: packages/engine/scripts/esp32s3-arduino.sh [--check] console" >&2
  echo "       packages/engine/scripts/esp32s3-arduino.sh [--check] knob LVGL_DIR" >&2
  exit 2
fi
name=$1
if [[ $name == knob && $# -ne 2 ]]; then
  echo "usage: packages/engine/scripts/esp32s3-arduino.sh [--check] knob LVGL_DIR" >&2
  exit 2
fi
if [[ $name != console && $name != knob ]]; then
  echo "esp32s3-arduino.sh: unknown sketch $name" >&2
  exit 2
fi
lvgl=
refs=
if [[ $name == knob ]]; then
  refs=$root/packages/engine/tests/firmware/esp32s3/arduino/checks/knob/refs.json
  if [[ $check -eq 1 && ! -f $refs ]]; then
    echo "esp32s3-arduino.sh: packages/engine/tests/firmware/esp32s3/arduino/checks/knob/refs.json is missing" >&2
    exit 1
  fi
  lvgl=$(cd "$2" && pwd)
fi

sdkconfig=$HOME/.arduino15/packages/esp32/tools/esp32s3-libs/3.3.12/qio_qspi/include/sdkconfig.h
sdkconfig_sha=f44c8812a67ab0c3c25704efd4aac3be2389119c65fc6179349aa7475a8405fa
console_elf_sha=143e2e2f6b5f77c152b48c5b2b516d66c9d7ce09a9671821a78dd0c1818be0b7
knob_elf_sha=cadfaa8b24a34ebfbb04e4ba36ee74e4104de31ef98220a2c94ec68dd2a176b8
lvgl_tree_sha=5734ad146cf77713982329c7712d30b765c761207b3caf5d55a431cf7ea3dc18
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

if [[ $name == knob ]]; then
  got_tree=$(python3 -I - "$lvgl" <<'PY'
import hashlib, os, sys
root = sys.argv[1]
files = []
for dirpath, dirnames, filenames in os.walk(root):
    dirnames.sort()
    for name in dirnames:
        if os.path.islink(os.path.join(dirpath, name)):
            sys.exit("esp32s3-arduino.sh: the LVGL tree has a symlink")
    for name in filenames:
        path = os.path.join(dirpath, name)
        if os.path.islink(path):
            sys.exit("esp32s3-arduino.sh: the LVGL tree has a symlink")
        files.append(os.path.relpath(path, root))
digest = hashlib.sha256()
for rel in sorted(files):
    digest.update(rel.encode())
    digest.update(b"\0")
    with open(os.path.join(root, rel), "rb") as fh:
        digest.update(fh.read())
    digest.update(b"\0")
print(f"{digest.hexdigest()} {len(files)}")
PY
)
  echo "lvgl tree: $got_tree"
  if [[ ${got_tree%% *} != "$lvgl_tree_sha" ]]; then
    echo "esp32s3-arduino.sh: the LVGL tree is ${got_tree%% *}, not $lvgl_tree_sha" >&2
    exit 1
  fi
fi

# An unset TMPDIR, and /tmp, would put the compile under /tmp. Use the repo scratch instead.
case ${TMPDIR:-} in
  ""|/tmp|/tmp/*)
    TMPDIR=$root/.scratch/arduino
    mkdir -p "$TMPDIR"
    export TMPDIR
    ;;
esac
work=$(mktemp -d "$TMPDIR/esp32s3-arduino.XXXXXX")
trap 'rm -rf "$work"' EXIT
sketch=$work/$name
mkdir -p "$sketch"
build=$work/build
out=$work/out
fqbn=esp32:esp32:esp32s3:CDCOnBoot=cdc,USBMode=hwcdc,FlashSize=16M,PSRAM=disabled
map="-ffile-prefix-map=$work=/src -ffile-prefix-map=$build=/build -ffile-prefix-map=$HOME/.arduino15=/arduino15"
if [[ $name == console ]]; then
  cp "$root/packages/engine/tests/firmware/esp32s3/arduino/console/console.ino" "$sketch/"
else
  cp "$root/packages/engine/tests/firmware/esp32s3/arduino/knob/knob.ino" "$root/packages/engine/tests/firmware/esp32s3/arduino/knob/wrap.c" "$sketch/"
  mkdir -p "$work/libraries/knobpanel/src" "$work/libraries/lvgl"
  cp -a "$lvgl"/. "$work/libraries/lvgl/"
  cp "$root/packages/engine/tests/firmware/esp32s3/arduino/libraries/lv_conf.h" "$work/libraries/lv_conf.h"
  cp -a "$root/packages/engine/tests/firmware/esp32s3/arduino/libraries/knobpanel"/. "$work/libraries/knobpanel/"
  cp "$root/packages/engine/tests/firmware/esp32s3/idf/screen/main/vendor_init.inc" "$work/libraries/knobpanel/src/vendor_init.inc"
fi
compile_args=(
  --build-property "compiler.c.extra_flags=$map"
  --build-property "compiler.cpp.extra_flags=$map"
  --build-property "compiler.S.extra_flags=$map"
)
if [[ $name == knob ]]; then
  compile_args+=(--libraries "$work/libraries")
  compile_args+=(--build-property "compiler.c.elf.extra_flags=-Wl,--wrap=esp_lcd_panel_io_tx_param -Wl,--wrap=esp_lcd_panel_io_tx_color -Wl,--wrap=esp_lcd_panel_draw_bitmap -Wl,--wrap=spi_device_queue_trans -Wl,--wrap=spi_device_polling_transmit -Wl,--wrap=spi_device_transmit")
fi
(
  cd "$work"
  SOURCE_DATE_EPOCH=1767225600 arduino-cli compile --clean --fqbn "$fqbn" --jobs 2 \
    --build-path "$build" --output-dir "$out" \
    "${compile_args[@]}" \
    "$sketch"
)

sha() {
  sha256sum "$1" | cut -d' ' -f1
}
bin=$out/$name.ino.bin
elf=$out/$name.ino.elf
table=$out/$name.ino.partitions.bin
bin_sha=$(sha "$bin")
table_sha=$(sha "$table")
got_elf=$(sha "$elf")
echo "$name.ino.bin: $bin_sha $(wc -c < "$bin")"
echo "$name.ino.elf: $got_elf $(wc -c < "$elf")"
echo "$name.ino.partitions.bin: $table_sha $(wc -c < "$table")"
elf_sha=$console_elf_sha
if [[ $name == knob ]]; then
  elf_sha=$knob_elf_sha
  python3 -I - "$bin" "$elf" "$HOME" "$work" <<'PY'
import sys
bin_path, elf_path, home, work = sys.argv[1:]
if home.encode() in open(bin_path, "rb").read():
    sys.exit("esp32s3-arduino.sh: the bin holds the home path")
if work.encode() in open(elf_path, "rb").read():
    sys.exit("esp32s3-arduino.sh: the ELF holds the build directory")
print("home path in bin: 0")
print("dir in elf: 0")
PY
fi
if [[ $got_elf != "$elf_sha" ]]; then
  echo "esp32s3-arduino.sh: the ELF is $got_elf, not $elf_sha" >&2
  exit 1
fi

netlist=$root/packages/engine/tests/firmware/esp32s3/arduino/checks/$name/netlist.json
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

cache=$( "$root/packages/engine/scripts/blobs.sh" | tail -n 1)
cp "$table" "$cache/$table_sha"
cp "$bin" "$cache/$bin_sha"
if [[ $name == knob ]]; then
  cp "$elf" "$cache/$elf_sha"
fi
if [[ $(sha "$cache/$table_sha") != "$table_sha" || $(sha "$cache/$bin_sha") != "$bin_sha" ]]; then
  echo "esp32s3-arduino.sh: the blob cache does not hold the built images" >&2
  exit 1
fi
if [[ $name == knob && $(sha "$cache/$elf_sha") != "$elf_sha" ]]; then
  echo "esp32s3-arduino.sh: the blob cache does not hold the ELF" >&2
  exit 1
fi

if ((check)); then
  run=${CARGO_TARGET_DIR:-$root/target}/esp32s3-arduino-$name-check
  rm -rf "$run"
  (
    cd "$root"
    echo "rustc $(rustc --version)"
    cargo run -q -p orbe-engine --features runner --bin orbe-run --locked -- \
      run "$netlist" --out "$run" --blobs "$cache"
  )
  if [[ $name == console ]]; then
    uv run "$root/packages/engine/scripts/esp32s3_arduino_console.py" --check "$run"
  else
    uv run "$root/packages/engine/scripts/esp32s3_knob.py" --check "$run" "$refs"
  fi
  rm -rf "$run"
fi
