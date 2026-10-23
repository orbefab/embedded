#!/usr/bin/env bash
# Rebuilds firmware/sketch/<name>.hex from firmware/sketch/<name>/<name>.ino with a pinned AVR
# toolchain and the standard AVR runtime, for the ATmega328P at 16 MHz, with the flags the
# standard IDE uses. The runtime's core is compiled into an archive and linked with each sketch,
# and main() comes from the core. A build of the engine reads only the committed hex.
#
# Usage: scripts/sketches.sh TOOLS_DIR [DOWNLOAD_DIR]
#
# Each pinned archive is fetched into DOWNLOAD_DIR (default TOOLS_DIR/dl), checked against its
# SHA-256, and unpacked into its own directory under TOOLS_DIR. A later run reuses both. An
# archive already unpacked under TOOLS_DIR is used as is, without checking its hash again.
set -euo pipefail

toolchain_url=https://downloads.arduino.cc/tools/avr-gcc-7.3.0-atmel3.6.1-arduino7-x86_64-pc-linux-gnu.tar.bz2
toolchain_sha256=bd8c37f6952a2130ac9ee32c53f6a660feb79bee8353c8e289eb60fdcefed91e
runtime_url=https://downloads.arduino.cc/cores/staging/avr-1.8.6.tar.bz2
runtime_sha256=ff1d17274b5a952f172074bd36c3924336baefded0232e10982f8999c2f7c3b6

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 TOOLS_DIR [DOWNLOAD_DIR]" >&2
  exit 2
fi
mkdir -p "$1"
tools=$(cd "$1" && pwd)
downloads=${2:-$tools/dl}
mkdir -p "$downloads"
firmware=$(cd "$(dirname "$0")/../firmware/sketch" && pwd)

unpack() {
  local dir=$1 url=$2 sha256=$3
  local archive=$downloads/${url##*/}
  [[ -d $tools/$dir ]] && return
  if [[ -f $archive ]]; then
    echo "$sha256  $archive" | sha256sum --check --quiet
  else
    curl -fsSL -o "$archive.part" "$url"
    echo "$sha256  $archive.part" | sha256sum --check --quiet
    mv "$archive.part" "$archive"
  fi
  rm -rf "$tools/$dir.part"
  mkdir "$tools/$dir.part"
  tar -xjf "$archive" -C "$tools/$dir.part"
  mv "$tools/$dir.part" "$tools/$dir"
}
unpack avr-gcc-7.3.0 "$toolchain_url" "$toolchain_sha256"
unpack avr-runtime-1.8.6 "$runtime_url" "$runtime_sha256"

bin=$tools/avr-gcc-7.3.0/avr/bin
runtime=$tools/avr-runtime-1.8.6/avr-1.8.6
core=$runtime/cores/arduino
target=(-mmcu=atmega328p -DF_CPU=16000000L -I"$core" -I"$runtime/variants/standard")
c_flags=(-c -g -Os -w -std=gnu11 -ffunction-sections -fdata-sections -flto -fno-fat-lto-objects)
cpp_flags=(-c -g -Os -w -std=gnu++11 -fpermissive -fno-exceptions -ffunction-sections
  -fdata-sections -fno-threadsafe-statics -Wno-error=narrowing -flto)
s_flags=(-c -g -x assembler-with-cpp -flto)
link_flags=(-w -Os -g -flto -fuse-linker-plugin -Wl,--gc-sections -mmcu=atmega328p)

work=$(mktemp -d "$tools/build.XXXXXX")
trap 'rm -rf "$work"' EXIT

mkdir "$work/core"
for source in $(cd "$core" && ls -- *.c *.cpp *.S | LC_ALL=C sort); do
  object=$work/core/$source.o
  case $source in
    *.c) "$bin/avr-gcc" "${c_flags[@]}" "${target[@]}" "$core/$source" -o "$object" ;;
    *.cpp) "$bin/avr-g++" "${cpp_flags[@]}" "${target[@]}" "$core/$source" -o "$object" ;;
    *.S) "$bin/avr-gcc" "${s_flags[@]}" "${target[@]}" "$core/$source" -o "$object" ;;
  esac
  "$bin/avr-gcc-ar" rcs "$work/core.a" "$object"
done

for dir in "$firmware"/*/; do
  dir=${dir%/}
  name=$(basename "$dir")
  "$bin/avr-g++" "${cpp_flags[@]}" "${target[@]}" -x c++ -include Arduino.h \
    "$dir/$name.ino" -o "$work/$name.o"
  "$bin/avr-gcc" "${link_flags[@]}" -o "$work/$name.elf" "$work/$name.o" "$work/core.a" -L"$work" -lm
  "$bin/avr-objcopy" -O ihex -R .eeprom "$work/$name.elf" "$firmware/$name.hex"
  "$bin/avr-size" "$work/$name.elf" | tail -n 1 | awk -v n="$name" '{print n ": " $1 + $2 " flash bytes"}'
done
