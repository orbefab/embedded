#!/usr/bin/env bash
# Rebuilds each sketch image from <dir>/<name>/<name>.ino, where <dir> is
# packages/engine/tests/firmware/sketch or a Block's blocks/<id>/firmware, with a pinned AVR
# toolchain and the standard AVR runtime, for the ATmega328P, with the flags the standard IDE
# uses. An engine sketch is written to <dir>/<name>.hex. A Block's image is written to
# blocks/<id>/build/<name>.ino.hex, the path arduino-cli compile --output-dir build writes
# for that sketch. The clock is 16 MHz unless the sketch directory contains `f_cpu`, one integer of hertz.
# The runtime's core is compiled into an archive and linked with each sketch, and main() comes
# from the core. A Block whose board is not nano-board or uno-board is skipped. The runtime's
# Wire library is compiled into a second archive, which a sketch that includes Wire.h also links. A sketch that includes Adafruit_NeoPixel.h also links that
# library, pinned below, compiled for the sketch's clock. A sketch that includes
# Adafruit_SSD1306.h also links Adafruit SSD1306, Adafruit GFX and the runtime's SPI library,
# and compiles against Adafruit BusIO's headers, which Adafruit_GFX.h includes. A sketch that
# includes Adafruit_BMP280.h also links that library, Adafruit BusIO, Adafruit Unified Sensor
# and the runtime's SPI library. A clock other than 16 MHz gets its own core and Wire archives.
# A build of the engine reads only the committed hex.
#
# Usage: packages/engine/scripts/sketches.sh TOOLS_DIR [DOWNLOAD_DIR]
#
# Each pinned archive is fetched into DOWNLOAD_DIR (default TOOLS_DIR/dl), checked against its
# SHA-256, and unpacked into its own directory under TOOLS_DIR. A later run reuses both. An
# archive already unpacked under TOOLS_DIR is used as is, without checking its hash again.
set -euo pipefail

toolchain_url=https://downloads.arduino.cc/tools/avr-gcc-7.3.0-atmel3.6.1-arduino7-x86_64-pc-linux-gnu.tar.bz2
toolchain_sha256=bd8c37f6952a2130ac9ee32c53f6a660feb79bee8353c8e289eb60fdcefed91e
runtime_url=https://downloads.arduino.cc/cores/staging/avr-1.8.6.tar.bz2
runtime_sha256=ff1d17274b5a952f172074bd36c3924336baefded0232e10982f8999c2f7c3b6
neopixel_url=https://github.com/adafruit/Adafruit_NeoPixel/archive/refs/tags/1.15.5.tar.gz
neopixel_sha256=3aa189a5a397a13539fcb58ff297ab9dc4a4faf45115e48afd533f1337cb0f3b
bmp280_url=https://github.com/adafruit/Adafruit_BMP280_Library/archive/refs/tags/3.0.0.tar.gz
bmp280_sha256=54018faa0cb89ef25b54c79b1ab274e4010e9b2e41632446c72ac6ceed8f1241
sensor_url=https://github.com/adafruit/Adafruit_Sensor/archive/refs/tags/1.1.15.tar.gz
sensor_sha256=1763c21c969a74d69e4d29082c7458a2d474d51e5ad0e2c4803dda8e1c372d9a
ssd1306_url=https://github.com/adafruit/Adafruit_SSD1306/archive/refs/tags/2.5.17.tar.gz
ssd1306_sha256=5228d4d2e915f6a4aa15b681bcd27a9f848627f7542283dd3cb3b76445a32ef9
gfx_url=https://github.com/adafruit/Adafruit-GFX-Library/archive/refs/tags/1.13.0.tar.gz
gfx_sha256=8ab038fb445d99f320feb9d463c7afe4cd91475fd8ea9c4047be444aacbb3b29
busio_url=https://github.com/adafruit/Adafruit_BusIO/archive/refs/tags/1.17.4.tar.gz
busio_sha256=59efa3acd5466e381c1b0313d5f0e824832db1bb24e4e328667e12b7d235d56d

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 TOOLS_DIR [DOWNLOAD_DIR]" >&2
  exit 2
fi
mkdir -p "$1"
tools=$(cd "$1" && pwd)
downloads=${2:-$tools/dl}
mkdir -p "$downloads"
repo=$(cd "$(dirname "$0")/../../.." && pwd)

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
  case $archive in
    *.tar.bz2) tar -xjf "$archive" -C "$tools/$dir.part" ;;
    *.tar.gz) tar -xzf "$archive" -C "$tools/$dir.part" ;;
    *)
      echo "sketches.sh: $url is not a .tar.bz2 or .tar.gz" >&2
      exit 1
      ;;
  esac
  mv "$tools/$dir.part" "$tools/$dir"
}
unpack avr-gcc-7.3.0 "$toolchain_url" "$toolchain_sha256"
unpack avr-runtime-1.8.6 "$runtime_url" "$runtime_sha256"
unpack adafruit-neopixel-1.15.5 "$neopixel_url" "$neopixel_sha256"
unpack adafruit-bmp280-3.0.0 "$bmp280_url" "$bmp280_sha256"
unpack adafruit-sensor-1.1.15 "$sensor_url" "$sensor_sha256"
unpack adafruit-ssd1306-2.5.17 "$ssd1306_url" "$ssd1306_sha256"
unpack adafruit-gfx-1.13.0 "$gfx_url" "$gfx_sha256"
unpack adafruit-busio-1.17.4 "$busio_url" "$busio_sha256"

bin=$tools/avr-gcc-7.3.0/avr/bin
runtime=$tools/avr-runtime-1.8.6/avr-1.8.6
neopixel=$tools/adafruit-neopixel-1.15.5/Adafruit_NeoPixel-1.15.5
bmp280=$tools/adafruit-bmp280-3.0.0/Adafruit_BMP280_Library-3.0.0
sensor=$tools/adafruit-sensor-1.1.15/Adafruit_Sensor-1.1.15
ssd1306=$tools/adafruit-ssd1306-2.5.17/Adafruit_SSD1306-2.5.17
gfx=$tools/adafruit-gfx-1.13.0/Adafruit-GFX-Library-1.13.0
busio=$tools/adafruit-busio-1.17.4/Adafruit_BusIO-1.17.4
spi=$runtime/libraries/SPI/src
core=$runtime/cores/arduino
target=(-mmcu=atmega328p -DF_CPU=16000000L -I"$core" -I"$runtime/variants/standard")
c_flags=(-c -g -Os -w -std=gnu11 -ffunction-sections -fdata-sections -flto -fno-fat-lto-objects)
cpp_flags=(-c -g -Os -w -std=gnu++11 -fpermissive -fno-exceptions -ffunction-sections
  -fdata-sections -fno-threadsafe-statics -Wno-error=narrowing -flto)
s_flags=(-c -g -x assembler-with-cpp -flto)
link_flags=(-w -Os -g -flto -fuse-linker-plugin "-Wl,--gc-sections" -mmcu=atmega328p)

work=$(mktemp -d "$tools/build.XXXXXX")
trap 'rm -rf "$work"' EXIT

# A pattern that matches nothing stays literal, so its compile fails by name.
mapfile -t core_sources < <(cd "$core" && printf '%s\n' *.c *.cpp *.S | LC_ALL=C sort)
mkdir "$work/core"
for source in "${core_sources[@]}"; do
  object=$work/core/$source.o
  case $source in
    *.c) "$bin/avr-gcc" "${c_flags[@]}" "${target[@]}" "$core/$source" -o "$object" ;;
    *.cpp) "$bin/avr-g++" "${cpp_flags[@]}" "${target[@]}" "$core/$source" -o "$object" ;;
    *.S) "$bin/avr-gcc" "${s_flags[@]}" "${target[@]}" "$core/$source" -o "$object" ;;
  esac
  "$bin/avr-gcc-ar" rcs "$work/core.a" "$object"
done

wire=$runtime/libraries/Wire/src
mkdir "$work/wire"
"$bin/avr-g++" "${cpp_flags[@]}" "${target[@]}" -I"$wire" -I"$wire/utility" "$wire/Wire.cpp" \
  -o "$work/wire/Wire.cpp.o"
"$bin/avr-gcc" "${c_flags[@]}" "${target[@]}" -I"$wire/utility" "$wire/utility/twi.c" \
  -o "$work/wire/twi.c.o"
"$bin/avr-gcc-ar" rcs "$work/wire.a" "$work/wire/Wire.cpp.o" "$work/wire/twi.c.o"

# A clock other than 16 MHz. The 16 MHz archives above stay the ones every other sketch links.
clock_archives() {
  local cpu=$1
  local dest=$work/cpu-$cpu
  [[ -f $dest/core.a ]] && return
  mkdir -p "$dest/core" "$dest/wire"
  local clocked=(-mmcu=atmega328p -DF_CPU="${cpu}L" -I"$core" -I"$runtime/variants/standard")
  local source object
  for source in "${core_sources[@]}"; do
    object=$dest/core/$source.o
    case $source in
      *.c) "$bin/avr-gcc" "${c_flags[@]}" "${clocked[@]}" "$core/$source" -o "$object" ;;
      *.cpp) "$bin/avr-g++" "${cpp_flags[@]}" "${clocked[@]}" "$core/$source" -o "$object" ;;
      *.S) "$bin/avr-gcc" "${s_flags[@]}" "${clocked[@]}" "$core/$source" -o "$object" ;;
    esac
    "$bin/avr-gcc-ar" rcs "$dest/core.a" "$object"
  done
  "$bin/avr-g++" "${cpp_flags[@]}" "${clocked[@]}" -I"$wire" -I"$wire/utility" "$wire/Wire.cpp" \
    -o "$dest/wire/Wire.cpp.o"
  "$bin/avr-gcc" "${c_flags[@]}" "${clocked[@]}" -I"$wire/utility" "$wire/utility/twi.c" \
    -o "$dest/wire/twi.c.o"
  "$bin/avr-gcc-ar" rcs "$dest/wire.a" "$dest/wire/Wire.cpp.o" "$dest/wire/twi.c.o"
}

for dir in "$repo"/packages/engine/tests/firmware/sketch/*/ "$repo"/blocks/*/firmware/*/; do
  dir=${dir%/}
  name=$(basename "$dir")
  firmware=$(dirname "$dir")
  # A Block on a board other than the Nano or the Uno is not an ATmega328P sketch.
  card=$(dirname "$firmware")/block.jsonc
  if [[ -f $card ]] && ! grep -Eq '"board": "(nano|uno)-board"' "$card"; then
    continue
  fi
  library=()
  archives=()
  sketch_target=("${target[@]}")
  core_a=$work/core.a
  wire_a=$work/wire.a
  if [[ -f $dir/f_cpu ]]; then
    cpu=$(tr -d '[:space:]' < "$dir/f_cpu")
    if [[ $cpu != 16000000 ]]; then
      clock_archives "$cpu"
      sketch_target=(-mmcu=atmega328p -DF_CPU="${cpu}L" -I"$core" -I"$runtime/variants/standard")
      core_a=$work/cpu-$cpu/core.a
      wire_a=$work/cpu-$cpu/wire.a
    fi
  fi
  if grep -q '^#include <Wire.h>' "$dir/$name.ino"; then
    library=(-I"$wire")
    archives=("$wire_a")
  fi
  if grep -q '^#include <Adafruit_NeoPixel.h>' "$dir/$name.ino"; then
    "$bin/avr-g++" "${cpp_flags[@]}" "${sketch_target[@]}" -DARDUINO=10806 -I"$neopixel" \
      "$neopixel/Adafruit_NeoPixel.cpp" -o "$work/$name-neopixel.o"
    library+=(-I"$neopixel" -DARDUINO=10806)
    archives+=("$work/$name-neopixel.o")
  fi
  if grep -q '^#include <Adafruit_BMP280.h>' "$dir/$name.ino"; then
    bmp_flags=(-I"$bmp280" -I"$busio" -I"$sensor" -I"$spi" -I"$wire" -DARDUINO=10806)
    "$bin/avr-g++" "${cpp_flags[@]}" "${sketch_target[@]}" "${bmp_flags[@]}" \
      "$bmp280/Adafruit_BMP280.cpp" -o "$work/$name-bmp280.o"
    "$bin/avr-g++" "${cpp_flags[@]}" "${sketch_target[@]}" "${bmp_flags[@]}" \
      "$busio/Adafruit_I2CDevice.cpp" -o "$work/$name-i2cdevice.o"
    "$bin/avr-g++" "${cpp_flags[@]}" "${sketch_target[@]}" "${bmp_flags[@]}" \
      "$busio/Adafruit_SPIDevice.cpp" -o "$work/$name-spidevice.o"
    "$bin/avr-g++" "${cpp_flags[@]}" "${sketch_target[@]}" "${bmp_flags[@]}" \
      "$sensor/Adafruit_Sensor.cpp" -o "$work/$name-sensor.o"
    "$bin/avr-g++" "${cpp_flags[@]}" "${sketch_target[@]}" -I"$spi" \
      "$spi/SPI.cpp" -o "$work/$name-spi.o"
    library+=("${bmp_flags[@]}")
    archives+=("$work/$name-bmp280.o" "$work/$name-i2cdevice.o" "$work/$name-spidevice.o" \
      "$work/$name-sensor.o" "$work/$name-spi.o")
  fi
  if grep -q '^#include <Adafruit_SSD1306.h>' "$dir/$name.ino"; then
    oled=(-DARDUINO=10806 -I"$ssd1306" -I"$gfx" -I"$busio" -I"$wire" -I"$spi")
    "$bin/avr-g++" "${cpp_flags[@]}" "${sketch_target[@]}" "${oled[@]}" \
      "$ssd1306/Adafruit_SSD1306.cpp" -o "$work/$name-ssd1306.o"
    "$bin/avr-g++" "${cpp_flags[@]}" "${sketch_target[@]}" "${oled[@]}" \
      "$gfx/Adafruit_GFX.cpp" -o "$work/$name-gfx.o"
    "$bin/avr-g++" "${cpp_flags[@]}" "${sketch_target[@]}" -I"$spi" "$spi/SPI.cpp" \
      -o "$work/$name-spi.o"
    library+=("${oled[@]}")
    archives+=("$work/$name-ssd1306.o" "$work/$name-gfx.o" "$work/$name-spi.o")
  fi
  "$bin/avr-g++" "${cpp_flags[@]}" "${sketch_target[@]}" "${library[@]}" -x c++ -include Arduino.h \
    "$dir/$name.ino" -o "$work/$name.o"
  "$bin/avr-gcc" "${link_flags[@]}" -o "$work/$name.elf" "$work/$name.o" "${archives[@]}" \
    "$core_a" -L"$work" -lm
  case $dir in
    "$repo"/blocks/*/firmware/*)
      out=$(dirname "$firmware")/build/$name.ino.hex
      mkdir -p "$(dirname "$out")"
      ;;
    *)
      out=$firmware/$name.hex
      ;;
  esac
  "$bin/avr-objcopy" -O ihex -R .eeprom "$work/$name.elf" "$out"
  "$bin/avr-size" "$work/$name.elf" | tail -n 1 | awk -v n="${dir#"$repo"/}" '{print n ": " $1 + $2 " flash bytes"}'
done
