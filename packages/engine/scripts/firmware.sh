#!/usr/bin/env bash
# Rebuilds packages/engine/tests/firmware/*.hex from packages/engine/tests/firmware/*.S. Needs avr-gcc and avr-objcopy on
# PATH. A build of the engine reads only the committed hex.
set -euo pipefail
cd "$(dirname "$0")/../tests/firmware"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for source in *.S; do
  name=${source%.S}
  avr-gcc -mmcu=atmega328p -nostartfiles -nostdlib -x assembler-with-cpp "$source" -o "$work/$name.elf"
  avr-objcopy -O ihex "$work/$name.elf" "$name.hex"
done
