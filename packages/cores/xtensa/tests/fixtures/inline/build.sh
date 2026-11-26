#!/usr/bin/env bash
# Assembles each program here into the .bin that tests/inline.rs reads: its .text from
# 0x40200000, the core's VECBASE, through its last byte. The reset PC is 0x40200400.
#
#   tests/fixtures/inline/build.sh
#
# It needs the S3's binutils on PATH, or their prefix in XTENSA_ESP32S3_PREFIX. The committed
# .bin files came from GNU binutils 2.43.1 of crosstool-NG esp-14.2.0_20260121.
set -euo pipefail
cd "$(dirname "$0")"
prefix=${XTENSA_ESP32S3_PREFIX:-xtensa-esp32s3-elf-}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for program in ops windows loops smc; do
  "${prefix}as" --no-transform -o "$work/$program.o" "$program.S"
  "${prefix}ld" -Ttext=0x40200000 -e start -o "$work/$program.elf" "$work/$program.o"
  "${prefix}objcopy" -O binary -j .text "$work/$program.elf" "$program.bin"
done
