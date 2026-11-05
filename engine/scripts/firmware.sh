#!/usr/bin/env bash
# Rebuilds firmware/*.hex from firmware/*.S. Needs avr-gcc and avr-objcopy on
# PATH. A build of the engine reads only the committed hex.
set -euo pipefail
cd "$(dirname "$0")/../firmware"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for source in *.S; do
  name=${source%.S}
  avr-gcc -mmcu=atmega328p -nostartfiles -nostdlib -x assembler-with-cpp "$source" -o "$work/$name.elf"
  avr-objcopy -O ihex "$work/$name.elf" "$name.hex"
done
