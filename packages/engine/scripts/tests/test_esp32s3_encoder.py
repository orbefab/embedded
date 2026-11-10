"""What `esp32s3_encoder` accepts and what it refuses."""

import json

from esp32s3_encoder import EXAMPLE, READY, US_PS, check_console, check_diagnostics, check_manifest, closings, lines


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


def test_edges_polls_and_manifest() -> None:

    def switch(name: str, closed: list[int]) -> dict:
        events = [e for t in closed for e in ({"t_ps": t, "state": "closed"}, {"t_ps": t + 9 * US_PS, "state": "open"})]
        return {"name": name, "kind": "switch", "initial": "open", "events": events}

    ms = 1000 * US_PS
    netlist = {"parts": [switch("sw_a", [200 * ms, 400 * ms]), switch("sw_b", [300 * ms])]}
    closed = closings(netlist)
    timed = [
        (5, "I (57) cpu_start: Unicore app\r\n"),
        (110 * ms, READY),
        (200 * ms + 30 * US_PS, "edge a=1 b=0 t=200001\r\n"),
        (204 * ms, "poll a=1 b=0\r\n"),
        (300 * ms + 30 * US_PS, "edge a=1 b=1 t=300001\r\n"),
        (304 * ms, "poll a=1 b=1\r\n"),
        (400 * ms + 30 * US_PS, "edge a=2 b=1 t=400001\r\n"),
        (404 * ms, "poll a=2 b=1\r\n"),
    ]

    def console(change=None) -> list[tuple[int, str]]:
        rows = [(at, change(line) if change else line) for at, line in timed]
        return lines(serial_of([row for row in rows if row[1]]))

    expect("the closings come in time order", closed == [(200 * ms, 0), (300 * ms, 1), (400 * ms, 0)])
    expect("a good run passes", not raises(lambda: check_console(console(), closed)))
    expect("a good run counts each contact", check_console(console(), closed) == (2, 1))
    expect(
        "a late edge fails",
        raises(lambda: check_console(console(lambda line: line.replace("t=300001", "t=300011")), closed)),
    )
    expect(
        "an early edge fails",
        raises(lambda: check_console(console(lambda line: line.replace("t=300001", "t=299999")), closed)),
    )
    expect(
        "a miscounted edge fails",
        raises(lambda: check_console(console(lambda line: line.replace("edge a=1 b=1", "edge a=2 b=0")), closed)),
    )
    expect(
        "a lost edge fails",
        raises(lambda: check_console(console(lambda line: "" if "t=400001" in line else line), closed)),
    )
    expect(
        "a lost poll fails",
        raises(lambda: check_console(console(lambda line: "" if line == "poll a=1 b=1\r\n" else line), closed)),
    )
    late = [(at + 4 * ms if line == "poll a=1 b=1\r\n" else at, line) for at, line in timed]
    expect("a late poll fails", raises(lambda: check_console(lines(serial_of(late)), closed)))
    expect(
        "a miscounted poll fails",
        raises(lambda: check_console(console(lambda line: line.replace("poll a=2", "poll a=3")), closed)),
    )
    expect(
        "a console without ready fails",
        raises(lambda: check_console(console(lambda line: "" if line == READY else line), closed)),
    )
    extra = netlist["parts"] + [switch("sw_c", [500 * ms])]
    expect("another switch is not a contact", closings({"parts": extra}) == closed)
    expect(
        "a third closing of sw_a fails",
        raises(
            lambda: check_console(
                console(),
                closings({"parts": [switch("sw_a", [200 * ms, 400 * ms, 600 * ms]), switch("sw_b", [300 * ms])]}),
            )
        ),
    )
    expect("an error fails", raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])))
    expect(
        "a warning passes",
        not raises(lambda: check_diagnostics([{"code": "chip.unmodeled_register", "severity": "warning"}])),
    )
    other = serial_of(timed).replace("USB_SERIAL_JTAG", "USART0", 1)
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
    committed = closings(json.loads((EXAMPLE / "netlist.json").read_text()))
    expect("the committed contacts close seven times", len(committed) == 7)
