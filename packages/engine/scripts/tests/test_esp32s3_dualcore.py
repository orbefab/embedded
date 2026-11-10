"""What `esp32s3_dualcore` accepts and what it refuses."""

from esp32s3_dualcore import CORES, TICKS, check_console, check_diagnostics, check_manifest, lines


def expect(name: str, ok: bool) -> None:
    assert ok, name


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def serial_of(text: str) -> str:
    rows = "".join(f"0,u1,USB_SERIAL_JTAG,{ord(c)}\n" for c in text)
    return "t_ps,part,usart,byte\n" + rows


def transcript(end: str = "\r\n", swap: bool = False) -> str:
    """Twenty ticks from each core, interleaved one line at a time, then both done lines."""
    lines_out = []
    order = (1, 0) if swap else (0, 1)
    for k in range(TICKS):
        for core in order:
            lines_out.append(f"core {core} tick {k}{end}")
    lines_out.append(f"done{end}")
    lines_out.append(f"done{end}")
    return "".join(lines_out)


def test_ticks_done_and_manifest() -> None:

    good = lines(serial_of("I (57) boot\r\n" + transcript()))
    expect("a good run passes", not raises(lambda: check_console(good)))
    bare = lines(serial_of(transcript(end="\n")))
    expect("a line feed passes", not raises(lambda: check_console(bare)))
    swapped = lines(serial_of(transcript(swap=True)))
    expect("the other core first passes", not raises(lambda: check_console(swapped)))
    lost = lines(serial_of(transcript().replace("core 1 tick 7\r\n", "", 1)))
    expect("a lost tick fails", raises(lambda: check_console(lost)))
    disorder = lines(serial_of(transcript().replace("core 0 tick 3\r\n", "core 0 tick 4\r\n", 1)))
    expect("a tick out of order fails", raises(lambda: check_console(disorder)))
    one_done = lines(serial_of(transcript().replace("done\r\n", "", 1)))
    expect("one done line fails", raises(lambda: check_console(one_done)))
    early = []
    for k in range(TICKS):
        if k == TICKS - 1:
            early.append("done\r\n")
        for core in CORES:
            early.append(f"core {core} tick {k}\r\n")
    early.append("done\r\n")
    expect("a done line before the last tick fails", raises(lambda: check_console(lines(serial_of("".join(early))))))
    owned = []
    for core in CORES:
        for k in range(TICKS):
            owned.append(f"core {core} tick {k}\r\n")
        owned.append("done\r\n")
    expect("a done after that core's ticks passes", not raises(lambda: check_console(lines(serial_of("".join(owned))))))
    expect("an error fails", raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])))
    expect(
        "a warning passes",
        not raises(lambda: check_diagnostics([{"code": "chip.unmodeled_register", "severity": "warning"}])),
    )
    other = serial_of(transcript()).replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(other)))
    part = {
        "kind": "esp32s3",
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
    }
    listed = {
        "blobs": [{"role": "rom", "sha256": "r", "bytes": 1}, {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8}]
    }
    expect("the manifest passes", not raises(lambda: check_manifest(listed, {"parts": [part]})))
    unplaced = {"blobs": [listed["blobs"][0], {**listed["blobs"][1], "offset": None}]}
    expect("a region without its offset fails", raises(lambda: check_manifest(unplaced, {"parts": [part]})))
