"""What `esp32s3_touch` accepts and what it refuses."""

import json

from esp32s3_touch import (
    EXAMPLE,
    FILTER_CFG,
    POLLS,
    check_diagnostics,
    check_manifest,
    console_reads,
    lines,
    point_bytes,
    points,
    transfers,
)


def expect(name: str, ok: bool) -> None:
    assert ok, name


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def serial_of(timed: list[tuple[int, str]]) -> str:
    rows = "".join(f"{at},u1,USB_SERIAL_JTAG,{ord(c)}\n" for at, line in timed for c in line)
    return "t_ps,part,usart,byte\n" + rows


def read_of(reg: int, data: list[int]) -> list[dict]:
    phases = [
        {"phase": "start"},
        {"phase": "address", "read": 0},
        {"phase": "data", "byte": reg, "read": 0},
        {"phase": "repeated_start"},
        {"phase": "address", "read": 1},
    ]
    phases += [{"phase": "data", "byte": byte, "read": 1} for byte in data]
    phases.append({"phase": "stop"})
    return phases


def test_console_bus_and_manifest() -> None:

    touched = [(291, 69), (16, 32)]
    timed = [
        (1, "id b4\r\n"),
        *[(2 + n, "poll f=0\r\n") for n in range(POLLS)],
        (20, "touch f=1 x=291 y=69\r\n"),
        (21, "touch f=1 x=16 y=32\r\n"),
    ]
    phases = read_of(0xA7, [0xB4])
    phases += [phase for _ in range(POLLS) for phase in read_of(0x02, [0, 0, 0, 0, 0])]
    phases += read_of(0x02, point_bytes(1, 291, 69))
    phases += read_of(0x02, point_bytes(1, 16, 32))

    def console(change=None):
        rows = [(at, change(line) if change else line) for at, line in timed]
        return lines(serial_of([row for row in rows if row[1]]))

    expect("a good console names the reads", console_reads(console(), touched)[-1] == ([0x02], point_bytes(1, 16, 32)))
    expect("a good bus matches the console", transfers(phases) == console_reads(console(), touched))
    expect(
        "a wrong x fails", raises(lambda: console_reads(console(lambda line: line.replace("x=291", "x=292")), touched))
    )
    expect(
        "a missing id fails",
        raises(lambda: console_reads(console(lambda line: "" if line.startswith("id") else line), touched)),
    )
    expect(
        "a touch with no finger fails",
        raises(lambda: console_reads(console(lambda line: line.replace("touch f=1", "touch f=0", 1)), touched)),
    )
    expect(
        "a poll with fingers fails",
        raises(lambda: console_reads(console(lambda line: line.replace("poll f=0", "poll f=1", 1)), touched)),
    )
    expect("a short bus differs", transfers(phases[:-2]) != console_reads(console(), touched))
    expect("an error fails", raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])))
    expect(
        "another kept register passes",
        not raises(
            lambda: check_diagnostics(
                [{"code": "chip.unmodeled_register", "severity": "warning", "data": {"address": 0x60013004}}]
            )
        ),
    )
    expect(
        "FILTER_CFG fails",
        raises(
            lambda: check_diagnostics(
                [{"code": "chip.unmodeled_register", "severity": "warning", "data": {"address": FILTER_CFG}}]
            )
        ),
    )
    part = {
        "kind": "esp32s3",
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
    }
    listed = {
        "blobs": [
            {"role": "rom", "sha256": "r", "bytes": 1},
            {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8},
        ]
    }
    expect("the manifest passes", not raises(lambda: check_manifest(listed, {"parts": [part]})))
    unplaced = {"blobs": [listed["blobs"][0], {**listed["blobs"][1], "offset": None}]}
    expect("a region without its offset fails", raises(lambda: check_manifest(unplaced, {"parts": [part]})))
    other = serial_of(timed).replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(other)))
    committed = points(json.loads((EXAMPLE / "netlist.json").read_text()))
    expect("the committed touches are the two points", committed == [(291, 69), (16, 32)])
