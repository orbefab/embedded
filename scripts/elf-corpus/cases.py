#!/usr/bin/env python3
"""Write one malformed ELF per file into the directory named by argv[1]."""

import sys
from pathlib import Path

EM_XTENSA = 94
EM_AVR = 83
ET_EXEC = 2
PT_LOAD = 1
SHT_PROGBITS = 1
SHT_SYMTAB = 2
SHT_STRTAB = 3
CODE = 0x40370000
DATA = 0x3FC88000
ROM = 0x40000000


def u16(value):
    return (value & 0xFFFF).to_bytes(2, "little")


def u32(value):
    return (value & 0xFFFFFFFF).to_bytes(4, "little")


def ident(machine, entry, phoff, phnum, shoff, shnum, phentsize=32, shentsize=40, klass=1, data=1, version=1):
    raw = bytearray(52)
    raw[0:4] = b"\x7fELF"
    raw[4] = klass
    raw[5] = data
    raw[6] = version
    raw[16:18] = u16(ET_EXEC)
    raw[18:20] = u16(machine)
    raw[20:24] = u32(1)
    raw[24:28] = u32(entry)
    raw[28:32] = u32(phoff)
    raw[32:36] = u32(shoff)
    raw[40:42] = u16(52)
    raw[42:44] = u16(phentsize)
    raw[44:46] = u16(phnum)
    raw[46:48] = u16(shentsize)
    raw[48:50] = u16(shnum)
    return raw


def phdr(offset, vaddr, paddr, filesz, memsz, kind=PT_LOAD):
    raw = bytearray(32)
    raw[0:4] = u32(kind)
    raw[4:8] = u32(offset)
    raw[8:12] = u32(vaddr)
    raw[12:16] = u32(paddr)
    raw[16:20] = u32(filesz)
    raw[20:24] = u32(memsz)
    raw[24:28] = u32(5)
    raw[28:32] = u32(4)
    return raw


def shdr(kind, addr, offset, size, link=0):
    raw = bytearray(40)
    raw[4:8] = u32(kind)
    raw[12:16] = u32(addr)
    raw[16:20] = u32(offset)
    raw[20:24] = u32(size)
    raw[24:28] = u32(link)
    return raw


def xtensa_load(
    segments, symbols=(), phentsize=32, shentsize=40, machine=EM_XTENSA, klass=1, data=1, version=1, phoff=52
):
    """segments are (addr, payload, memsz). One program header each, then symbols."""
    payload = bytearray()
    headers = bytearray()
    data_at = phoff + phentsize * len(segments)
    for addr, body, memsz in segments:
        headers += phdr(data_at + len(payload), addr, addr, len(body), memsz)
        if phentsize > 32:
            headers += bytes(phentsize - 32)
        payload += body
    strings = bytearray(b"\0")
    table = bytearray(16)
    for name, addr in symbols:
        strings += name.encode() + b"\0"
        table += u32(len(strings) - len(name) - 1)
        table += u32(addr)
        table += u32(0)
        table += bytes((0x12, 0, 1, 0))
    raw = bytearray(
        ident(
            machine,
            segments[0][0] if segments else 0,
            phoff,
            len(segments),
            0,
            0,
            phentsize,
            shentsize,
            klass,
            data,
            version,
        )
    )
    raw += headers
    raw += payload
    if symbols:
        table_at = len(raw)
        raw += table
        strings_at = len(raw)
        raw += strings
        shoff = len(raw)
        raw[32:36] = u32(shoff)
        raw[48:50] = u16(3)
        raw += shdr(0, 0, 0, 0)
        raw += shdr(SHT_SYMTAB, 0, table_at, len(table), 2)
        raw += shdr(SHT_STRTAB, 0, strings_at, len(strings))
        if shentsize > 40:
            # The extra stride bytes sit after each 40-byte header the walk reads.
            padded = bytearray()
            for index in range(3):
                padded += raw[shoff + index * 40 : shoff + (index + 1) * 40]
                padded += bytes(shentsize - 40)
            raw = raw[:shoff] + padded
    return bytes(raw)


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    good = xtensa_load([(CODE, b"\x11\x22\x33\x44", 4)], [("main", CODE)])
    cases = {
        "engine-good": good,
        "engine-trunc-16": good[:16],
        "engine-trunc-51": good[:51],
        "engine-trunc-60": good[:60],
        "engine-wrong-class": xtensa_load([(CODE, b"\x11\x22\x33\x44", 4)], klass=2),
        "engine-wrong-machine": xtensa_load([(CODE, b"\x11\x22\x33\x44", 4)], machine=3),
        "engine-version": xtensa_load([(CODE, b"\x11\x22\x33\x44", 4)], version=0),
        "engine-big-endian": xtensa_load([(CODE, b"\x11\x22\x33\x44", 4)], data=2),
        "engine-overlap": xtensa_load([(CODE, b"\0\0\0\0", 0x8004), (DATA, b"\0\0\0\0", 4)]),
        "engine-outside": xtensa_load([(0x20000000, b"\0\0\0\0", 4)]),
        "engine-no-load": xtensa_load([]),
        "engine-phdr-wide": xtensa_load([(CODE, b"\x11\x22\x33\x44", 4)], phentsize=48),
        "engine-phdr-narrow": xtensa_load([(CODE, b"\x11\x22\x33\x44", 4)], phentsize=16),
        "engine-bss": xtensa_load([(CODE, b"\x11\x22\x33\x44", 8)]),
        "engine-shentsize-wide": xtensa_load([(CODE, b"\x11\x22\x33\x44", 4)], [("main", CODE)], shentsize=64),
        "engine-not-elf": b"MZ",
        "avr-good": avr([(0, b"\x11\x22"), (0x00800060, b"\xaa"), (0x00810000, b"\xbb\xcc")]),
        "avr-short": avr([(0, b"\x11\x22")])[:16],
        "avr-not-elf": b"MZ",
        "avr-class": avr([(0, b"\x11\x22")], klass=2),
        "avr-endian": avr([(0, b"\x11\x22")], data=2),
        "avr-machine": avr([(0, b"\x11\x22")], machine=62),
        "avr-phentsize-narrow": avr([(0, b"\x11\x22")], phentsize=16),
        "avr-phentsize-wide": avr([(0, b"\x11\x22")], phentsize=48),
        "avr-memsz": avr_memsz_below(),
        "avr-outside": avr([(0x00900000, b"\x11")]),
        "avr-past-file": avr_past_file(),
        "avr-phdr-past": avr_phdr_past(),
        "avr-cross": avr([(0x007FFFF0, b"\x11\x22")], memsz=32),
        "rom-sections": rom_sections(),
        "rom-past-file": rom_past_file(),
        "rom-past-end": rom_past_end(),
        "rom-not-elf": b"MZ",
        "rom-trunc": b"\x7fELF" + bytes(8),
        "rom-class": rom_sections(klass=2),
        "rom-endian": rom_sections(data=2),
        "rom-version": rom_sections(version=0),
        "rom-shentsize-wide": rom_sections(shentsize=64),
        "rom-shoff-zero": rom_shoff_zero(),
        "engine-phoff-zero": phoff_zero(),
        "engine-shoff-past": shoff_past(good),
        "engine-pn-xnum": pn_xnum(good),
        "engine-bad-strtab": bad_strtab(),
    }
    for name, body in cases.items():
        (out / name).write_bytes(body)


def avr(segments, machine=EM_AVR, klass=1, data=1, version=1, phentsize=32, memsz=None):
    payload = bytearray()
    headers = bytearray()
    phoff = 52
    data_at = phoff + phentsize * len(segments)
    for paddr, body in segments:
        size = len(body) if memsz is None else memsz
        headers += phdr(data_at + len(payload), paddr, paddr, len(body), size)
        if phentsize > 32:
            headers += bytes(phentsize - 32)
        payload += body
    raw = ident(machine, 0, phoff, len(segments), 0, 0, phentsize, 40, klass, data, version)
    return bytes(raw + headers + payload)


def avr_memsz_below():
    raw = bytearray(avr([(0, b"\x11\x22")]))
    raw[52 + 20 : 52 + 24] = u32(1)
    return bytes(raw)


def avr_past_file():
    raw = bytearray(avr([(0, b"\x11\x22")]))
    raw[52 + 4 : 52 + 8] = u32(0xFFFFFFF0)
    return bytes(raw)


def avr_phdr_past():
    return bytes(ident(EM_AVR, 0, 1000, 1, 0, 0))


def rom_sections(klass=1, data=1, version=1, shentsize=40):
    payload = b"\xab\xcd\xef\x01" + b"\x22\x33" + b"\x44\x55\x66\x77"
    # Headers first, then payload, so offsets are known.
    headers = [
        shdr(0, 0, 0, 0),
        shdr(SHT_PROGBITS, ROM, 0, 4),
        shdr(SHT_PROGBITS, 0x3FF00000, 0, 2),
        shdr(SHT_PROGBITS, DATA, 0, 4),
    ]
    shoff = 52
    raw = bytearray(ident(EM_XTENSA, 0, 0, 0, shoff, len(headers), 32, shentsize, klass, data, version))
    blob = bytearray()
    for header in headers:
        blob += header
        if shentsize > 40:
            blob += bytes(shentsize - 40)
    payload_at = shoff + len(blob)
    # Fill the offsets now that the payload position is known.
    spans = (4, 2, 4)
    cursor = payload_at
    for index, size in enumerate(spans, start=1):
        at = index * shentsize
        blob[at + 16 : at + 20] = u32(cursor)
        cursor += size
    raw += blob
    raw += payload
    return bytes(raw)


def rom_past_file():
    raw = bytearray(rom_sections())
    # First PROGBITS is section 1. Point its file offset past the end.
    raw[52 + 40 + 16 : 52 + 40 + 20] = u32(0x00FFFFF0)
    return bytes(raw)


def rom_past_end():
    """Four bytes at the last ROM address but one, so the section runs past the window."""
    raw = bytearray(ident(EM_XTENSA, 0, 0, 0, 52, 2))
    raw += shdr(0, 0, 0, 0)
    raw += shdr(SHT_PROGBITS, 0x4005FFFE, 52 + 80, 4)
    raw += b"\x01\x02\x03\x04"
    return bytes(raw)


def rom_shoff_zero():
    """Section headers at file offset 0. Section 2 is a real PROGBITS past the ident."""
    raw = bytearray(ident(EM_XTENSA, 0, 0, 0, 0, 3, shentsize=40))
    raw += bytes(28)  # rest of the header-sized hole through offset 80
    assert len(raw) == 80
    payload_at = 80 + 40
    raw += shdr(SHT_PROGBITS, ROM, payload_at, 4)
    raw += b"\x10\x20\x30\x40"
    return bytes(raw)


def phoff_zero():
    """Two program headers starting at 0. The second is a PT_LOAD the raw walk would see.

    p_vaddr 0x00200000 keeps e_phentsize at 32. p_paddr 2 keeps e_phnum at 2.
    """
    raw = bytearray(64)
    raw[:52] = ident(EM_XTENSA, CODE, 0, 2, 0, 0)
    raw[32:64] = phdr(64, 0x00200000, 2, 0, 4)
    return bytes(raw)


def shoff_past(good):
    raw = bytearray(good)
    raw[32:36] = u32(0x00100000)
    raw[46:48] = u16(40)
    raw[48:50] = u16(1)
    return bytes(raw)


def pn_xnum(good):
    raw = bytearray(good)
    raw[44:46] = u16(0xFFFF)
    return bytes(raw)


def bad_strtab():
    raw = bytearray(xtensa_load([(CODE, b"\x11\x22\x33\x44", 4)], [("main", CODE)]))
    # String table is section 2. Its sh_offset sits 16 bytes into that header.
    shoff = int.from_bytes(raw[32:36], "little")
    at = shoff + 2 * 40 + 16
    raw[at : at + 4] = u32(0x00FFFFF0)
    return bytes(raw)


if __name__ == "__main__":
    main()
