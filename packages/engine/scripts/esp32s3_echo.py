# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-echo` against the USB Serial/JTAG wake, apart from the engine.

    uv run packages/engine/scripts/esp32s3_echo.py --check RUN_DIR
    uv run packages/engine/scripts/esp32s3_echo.py --self-test

The app installs the USB Serial/JTAG driver, waits one FreeRTOS tick, reads `esp_timer_get_time`
as `t` and prints `ready <t>`. It then echoes each console byte as `got <b>`. `t` is microseconds
from the run's start. The netlist sends bytes at `a = t + n·1000 + 500` µs. With `n` the whole
thousands in `a - t`, the rest must lie from 100 to 700 µs, so a later change that moves `ready`
fails as a misplaced byte. Each `got` packet must come at or after `a`, by at most 100 µs, and before the
earliest the next tick can be, `t - 100 + (n + 1)·1000` µs.

`traces/serial.csv` must hold those lines as `u1`'s `USB_SERIAL_JTAG`. Other lines may sit
between them. The run must hold no error, and its `manifest.json` must list the netlist's ROM,
then each flash region with its offset.
"""

import argparse
import csv
import io
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EXAMPLE = ROOT / "packages/engine/tests/circuits" / "esp32s3-echo"
READY = re.compile(r"ready (\d+)\r?\n")
GOT = re.compile(r"got (\d+)\r?\n")
US = 10**6
MS = 1000 * US


def lines(serial_csv: str) -> list[tuple[int, str]]:
    """Each console line with the time of its first byte, in picoseconds."""
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    if any(row["part"] != "u1" or row["usart"] != "USB_SERIAL_JTAG" for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG")
    out, line, at = [], "", 0
    for row in rows:
        if not line:
            at = int(row["t_ps"])
        line += chr(int(row["byte"]))
        if line.endswith("\n"):
            out.append((at, line))
            line = ""
    if line:
        out.append((at, line))
    return out


def ready_us(got: list[tuple[int, str]]) -> int:
    """The `t` printed on the first `ready` line, in microseconds."""
    for _, line in got:
        match = READY.fullmatch(line)
        if match:
            return int(match.group(1))
    raise ValueError("the console has no ready line")


def got_packets(got: list[tuple[int, str]]) -> list[tuple[int, int]]:
    """Each `got <b>` packet: the time of its first byte, in picoseconds, and the byte."""
    packets = []
    for at, line in got:
        match = GOT.fullmatch(line)
        if match:
            packets.append((at, int(match.group(1))))
    return packets


def events(netlist: dict) -> list[tuple[int, int]]:
    """Each console byte and the time it is sent, in picoseconds."""
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    out = []
    for event in part.get("console", []):
        for byte in event["bytes"]:
            out.append((int(event["t_ps"]), int(byte)))
    return out


def check_timing(t_us: int, sent: list[tuple[int, int]], packets: list[tuple[int, int]]) -> None:
    """Each byte sits 100 to 700 µs after a millisecond, and its echo meets both bounds."""
    if len(packets) != len(sent):
        raise ValueError(f"the console echoed {len(packets)} bytes, not {len(sent)}")
    for (a_ps, byte), (got_ps, echoed) in zip(sent, packets):
        if echoed != byte:
            raise ValueError(f"got {echoed}, not {byte}")
        if a_ps % US != 0:
            raise ValueError(f"byte {byte} is sent at {a_ps} ps, not a whole microsecond")
        delta = a_ps // US - t_us
        n, rest = divmod(delta, 1000)
        if not 100 <= rest <= 700:
            raise ValueError(f"byte {byte} is misplaced: {rest} µs past millisecond {n}")
        if got_ps < a_ps or got_ps - a_ps > 100 * US:
            raise ValueError(f"got {byte} is {(got_ps - a_ps) // US} µs from its byte")
        earliest = (t_us - 100 + (n + 1) * 1000) * US
        if got_ps >= earliest:
            raise ValueError(f"got {byte} is at or after the next tick")


def check_manifest(manifest: dict, netlist: dict) -> None:
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    want = [{"role": "rom", **part["rom"]}]
    want += [
        {"role": "flash", "sha256": region["sha256"], "bytes": region["bytes"], "offset": region["offset"]}
        for region in part["firmware"]["flash"]
    ]
    if manifest["blobs"] != want:
        raise ValueError(f"manifest.json lists {manifest['blobs']}, not {want}")


def check_diagnostics(diagnostics: list[dict]) -> None:
    errors = [item["code"] for item in diagnostics if item["severity"] == "error"]
    if errors:
        raise ValueError(f"the run holds errors {errors}")


def check(run: Path) -> None:
    netlist = json.loads((EXAMPLE / "netlist.json").read_text())
    got = lines((run / "traces" / "serial.csv").read_text())
    check_timing(ready_us(got), events(netlist), got_packets(got))
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    print(f"esp32s3-echo: ready, then {len(events(netlist))} bytes echoed inside the tick")


def serial_of(text: str, at: int = 0) -> str:
    rows = "".join(f"{at},u1,USB_SERIAL_JTAG,{ord(c)}\n" for c in text)
    return "t_ps,part,usart,byte\n" + rows


def self_test() -> int:
    failures = 0

    def expect(name: str, ok: bool) -> None:
        nonlocal failures
        if not ok:
            print(f"FAIL {name}")
            failures += 1

    def raises(fn) -> bool:
        try:
            fn()
        except ValueError:
            return True
        return False

    t_us = 1_000_000
    n, rest, byte = 2, 500, 65
    a_us = t_us + n * 1000 + rest
    a_ps = a_us * US
    sent = [(a_ps, byte)]
    earliest = (t_us - 100 + (n + 1) * 1000) * US

    def packet(got_ps: int) -> list[tuple[int, int]]:
        return [(got_ps, byte)]

    expect("an echo at the byte passes", not raises(lambda: check_timing(t_us, sent, packet(a_ps))))
    expect(
        "an echo 100 µs later passes",
        not raises(lambda: check_timing(t_us, sent, packet(a_ps + 100 * US))),
    )
    early = packet(a_ps - 90 * US)
    expect("an echo 90 µs before its byte fails", raises(lambda: check_timing(t_us, sent, early)))
    late = packet(earliest + US)
    expect("an echo 1 µs after the next tick fails", raises(lambda: check_timing(t_us, sent, late)))
    misplaced = [(t_us + n * 1000 + 900) * US, byte]
    expect(
        "a byte 900 µs past the millisecond fails",
        raises(lambda: check_timing(t_us, [misplaced], packet(misplaced[0]))),
    )
    expect("the wrong byte fails", raises(lambda: check_timing(t_us, sent, [(a_ps, byte + 1)])))
    text = serial_of(f"ready {t_us}\n") + serial_of(f"got {byte}\n", a_ps).split("\n", 1)[1]
    parsed = lines(text)
    expect("ready is the printed t", ready_us(parsed) == t_us)
    expect("got keeps its byte", got_packets(parsed) == [(a_ps, byte)])
    expect("a missing ready line fails", raises(lambda: ready_us(lines(serial_of("boot\n")))))
    expect(
        "an error fails",
        raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])),
    )
    expect(
        "a warning passes",
        not raises(lambda: check_diagnostics([{"code": "chip.unmodeled_register", "severity": "warning"}])),
    )
    other = serial_of(f"ready {t_us}\n").replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(other)))
    part = {
        "kind": "esp32s3",
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
        "console": [{"t_ps": a_ps, "bytes": [byte]}],
    }
    listed = {
        "blobs": [
            {"role": "rom", "sha256": "r", "bytes": 1},
            {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8},
        ]
    }
    expect("the manifest passes", not raises(lambda: check_manifest(listed, {"parts": [part]})))
    expect("the console event is the byte", events({"parts": [part]}) == sent)
    print("self-test passed" if not failures else f"self-test: {failures} failed")
    return 1 if failures else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--check", type=Path, metavar="RUN_DIR")
    group.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"esp32s3-echo: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
