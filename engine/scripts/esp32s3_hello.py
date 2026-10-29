# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `examples/esp32s3-hello` against the board's console, apart from the engine.

    uv run scripts/esp32s3_hello.py --check RUN_DIR
    uv run scripts/esp32s3_hello.py --self-test

`examples/esp32s3-hello/console.json` holds the console text a board printed for the example's app,
from its first log line through `Restarting now.`, with each IDF log time masked to `T`. The
board's log times count from its power-on, which the run starts after, so only the masked text
compares.

`traces/serial.csv` must hold that text, byte for byte once masked, as `u1`'s `USB_SERIAL_JTAG`.
The app prints `Restarting in N seconds...` once a second, through FreeRTOS ticks of 10 ms, so
every adjacent pair of those lines, the first pair included, must lie one second apart, within a
tick. The
run's last diagnostic must be `chip.restart`, one second after the last such line within a tick,
and the run must hold no error.
Its `manifest.json` must list the netlist's ROM, then each flash region with its offset.
"""

import argparse
import csv
import io
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXAMPLE = ROOT / "examples" / "esp32s3-hello"
CONSOLE = EXAMPLE / "console.json"
LOG_TIME = re.compile(r"^([IWE]) \(\d+\) ", re.M)
COUNTDOWN = re.compile(r"Restarting in \d+ seconds\.\.\.\r\n")
SECOND_PS = 10**12
TICK_PS = 10**10


def masked(text: str) -> str:
    return LOG_TIME.sub(r"\1 (T) ", text)


def lines(serial_csv: str) -> list[tuple[int, str]]:
    """Each console line with the time of its first byte."""
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


def check_text(got: list[tuple[int, str]], want: str) -> None:
    text = masked("".join(line for _, line in got))
    if text != want:
        at = next((i for i, (a, b) in enumerate(zip(text, want)) if a != b), min(len(text), len(want)))
        raise ValueError(f"the console differs from the board's at byte {at}: {text[at:at + 60]!r}")


def check_times(got: list[tuple[int, str]], diagnostics: list[dict]) -> int:
    """The countdown lines a second apart, then the restart. Returns the countdown's length."""
    stamps = [at for at, line in got if COUNTDOWN.fullmatch(line)]
    errors = [d["code"] for d in diagnostics if d["severity"] == "error"]
    if errors:
        raise ValueError(f"the run holds errors {errors}")
    if not diagnostics or diagnostics[-1]["code"] != "chip.restart":
        raise ValueError("the run does not end with chip.restart")
    stamps.append(diagnostics[-1]["t_ps"])
    for before, after in zip(stamps, stamps[1:]):
        if abs(after - before - SECOND_PS) > TICK_PS:
            raise ValueError(f"{after - before} ps between two countdown steps, not a second")
    return len(stamps) - 1


def check_manifest(manifest: dict, netlist: dict) -> None:
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    want = [{"role": "rom", **part["rom"]}]
    want += [{"role": "flash", "sha256": r["sha256"], "bytes": r["bytes"], "offset": r["offset"]}
             for r in part["firmware"]["flash"]]
    if manifest["blobs"] != want:
        raise ValueError(f"manifest.json lists {manifest['blobs']}, not {want}")


def check(run: Path) -> None:
    want = json.loads(CONSOLE.read_text())["transcript"]
    got = lines((run / "traces" / "serial.csv").read_text())
    check_text(got, want)
    steps = check_times(got, json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), json.loads((EXAMPLE / "netlist.json").read_text()))
    print(f"esp32s3-hello: {len(want)} console bytes as the board printed them, {steps} countdown lines a second apart, then chip.restart")


def serial_of(timed: list[tuple[int, str]]) -> str:
    rows = "".join(f"{at},u1,USB_SERIAL_JTAG,{ord(c)}\n" for at, line in timed for c in line)
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

    want = "I (T) cpu_start: Unicore app\r\nRestarting in 1 seconds...\r\nRestarting in 0 seconds...\r\nRestarting now.\r\n"
    timed = [
        (5, "I (57) cpu_start: Unicore app\r\n"),
        (7 * TICK_PS, "Restarting in 1 seconds...\r\n"),
        (7 * TICK_PS + SECOND_PS, "Restarting in 0 seconds...\r\n"),
        (7 * TICK_PS + 2 * SECOND_PS, "Restarting now.\r\n"),
    ]
    restart = [{"code": "chip.restart", "severity": "warning", "t_ps": 7 * TICK_PS + 2 * SECOND_PS}]
    got = lines(serial_of(timed))
    expect("a good run passes the text", not raises(lambda: check_text(got, want)))
    expect("a good run passes the times", not raises(lambda: check_times(got, restart)))
    expect("log times are masked", masked("W (12) x\r\nI (3) y\r\n") == "W (T) x\r\nI (T) y\r\n")
    expect("a changed byte fails", raises(lambda: check_text(got, want.replace("Unicore", "Unicorn"))))
    expect("a lost carriage return fails", raises(lambda: check_text(got, want.replace("\r", ""))))
    late = list(timed)
    late[2] = (late[2][0] + 2 * TICK_PS, late[2][1])
    expect("a late countdown line fails", raises(lambda: check_times(lines(serial_of(late)), restart)))
    first = list(timed)
    first[1] = (first[1][0] + 2 * TICK_PS, first[1][1])
    expect("a late first countdown line fails", raises(lambda: check_times(lines(serial_of(first)), restart)))
    expect("an empty console fails", raises(lambda: check_text([], want)))
    expect("a truncated console fails", raises(lambda: check_text(got[:-1], want)))
    expect("a reordered console fails", raises(lambda: check_text([got[0], got[2], got[1], got[3]], want)))
    early = [{**restart[0], "t_ps": restart[0]["t_ps"] - 2 * TICK_PS}]
    expect("an early restart fails", raises(lambda: check_times(got, early)))
    expect("a run without the restart fails", raises(lambda: check_times(got, [])))
    error = [{"code": "chip.clock", "severity": "error", "t_ps": 0}, *restart]
    expect("an error fails", raises(lambda: check_times(got, error)))
    other = serial_of(timed).replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(other)))
    netlist = {"parts": [{"kind": "esp32s3", "rom": {"sha256": "r", "bytes": 1},
                          "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]}}]}
    listed = {"blobs": [{"role": "rom", "sha256": "r", "bytes": 1},
                        {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8}]}
    expect("the manifest passes", not raises(lambda: check_manifest(listed, netlist)))
    unplaced = {"blobs": [listed["blobs"][0], {**listed["blobs"][1], "offset": None}]}
    expect("a region without its offset fails", raises(lambda: check_manifest(unplaced, netlist)))
    expect("the committed console holds the countdown",
           len(COUNTDOWN.findall(json.loads(CONSOLE.read_text())["transcript"])) == 11)
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
    except (ValueError, OSError, KeyError) as error:
        print(f"esp32s3-hello: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
