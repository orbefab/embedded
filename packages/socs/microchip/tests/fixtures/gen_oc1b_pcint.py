#!/usr/bin/env python3
"""OC1B compare and the pin-change handler, as Intel HEX.

The program, in words:

Reset jumps to main. PCINT0 jumps to a handler. TIMER1_COMPB is RETI, so
that vector does not toggle a pin.

main sets DDRB, enables PCIE0 and PCINT2 (PB2), sets ICR1 to 0x9F and
OCR1B to 0, toggles OC1B (COM1B0), enables OCIE1B, and runs Timer1 in CTC
mode 12 at clk/64. It then selects idle sleep and enables interrupts.

The loop sleeps, does one store, and jumps back.
  eear: store 0 to EEARL. DDRB is PB2 and PB5. The handler is SBI PINB,5.
  gpior: store 0 to GPIOR0, then a nop. Same pins and handler as eear.

Usage:
  python3 gen_oc1b_pcint.py eear oc1b_pcint_eear.hex
  python3 gen_oc1b_pcint.py gpior oc1b_pcint_gpior.hex
"""

import sys


def ldi(d, k):
    return 0xE000 | ((k & 0xF0) << 4) | ((d - 16) << 4) | (k & 0x0F)


def out(a, r):
    return 0xB800 | ((a & 0x30) << 5) | (r << 4) | (a & 0x0F)


def sts(k, r):
    return [0x9200 | (r << 4), k]


def program(kind):
    words = [0] * 0x80
    words[0] = 0x940C
    words[1] = 0x40
    words[6] = 0x940C
    words[7] = 0x70
    words[0x18] = 0x9518
    main = [ldi(16, 0x24), out(0x04, 16), ldi(16, 1)] + sts(0x68, 16)
    main += [ldi(16, 4)] + sts(0x6B, 16)
    main += [ldi(16, 0)] + sts(0x87, 16) + [ldi(16, 0x9F)] + sts(0x86, 16)
    main += [ldi(16, 0)] + sts(0x8B, 16) + sts(0x8A, 16)
    main += [ldi(16, 0x10)] + sts(0x80, 16)
    main += [ldi(16, 0x04)] + sts(0x6F, 16)
    main += [ldi(16, 0x1B)] + sts(0x81, 16)
    main += [ldi(16, 1), out(0x33, 16), 0x9478]
    loop_start = 0x40 + len(main)
    main += [0x9588]
    if kind == "eear":
        main += [ldi(17, 0)] + sts(0x41, 17)
    else:
        main += [ldi(17, 0), out(0x1E, 17), 0]
    main.append(0xC000 | ((loop_start - (0x40 + len(main) + 1)) & 0xFFF))
    words[0x40 : 0x40 + len(main)] = main
    words[0x70] = 0x9A1D
    words[0x71] = 0x9518
    return words


def ihex(words):
    blob = b"".join(word.to_bytes(2, "little") for word in words)
    lines = []
    for offset in range(0, len(blob), 16):
        chunk = blob[offset : offset + 16]
        total = len(chunk) + (offset >> 8) + (offset & 0xFF) + sum(chunk)
        lines.append(":%02X%04X00%s%02X" % (len(chunk), offset, chunk.hex().upper(), (-total) & 0xFF))
    lines.append(":00000001FF")
    return "\n".join(lines) + "\n"


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in ("eear", "gpior"):
        sys.exit("usage: gen_oc1b_pcint.py eear|gpior OUT.hex")
    with open(sys.argv[2], "w", encoding="ascii") as out_file:
        out_file.write(ihex(program(sys.argv[1])))


if __name__ == "__main__":
    main()
