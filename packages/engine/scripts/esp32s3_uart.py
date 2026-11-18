# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-uart` apart from the engine.

    uv run packages/engine/scripts/esp32s3_uart.py --check RUN_DIR

The app waits for `g`, then for 115200 and 1000000 baud sends the bytes named in
`uart_main.c`, reads each back through the pad, and prints each edge of GPIO43 as
`edge <offset> <level>`. The offset is the `ccount` after the `GPIO_IN1` load that
saw the new level, minus the `ccount` read in the instruction before the FIFO store.
A printed offset lies in `[edge, edge + EDGE_POLL_BOUND)`.

The bytes and the bit lengths come from that file and from the divider it writes.
`traces/serial.csv` holds the console on `USB_SERIAL_JTAG` and each UART byte, sent and
received, on `UART0`. The run must hold no error, and `manifest.json` must list the ROM,
then each flash region with its offset.

`packages/engine/tests/circuits/esp32s3-uart/board.txt`, when it is present, is a console
capture of the same app. Each `edge` offset in the run must lie within `BOARD_TOLERANCE`
CPU cycles of the capture's matching edge. A missing file skips that compare.
"""

import argparse
import csv
import io
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EXAMPLE = ROOT / "packages/engine/tests/circuits" / "esp32s3-uart"
SOURCE = ROOT / "packages/engine/tests/firmware/esp32s3/idf/uart/main/uart_main.c"
BOARD = EXAMPLE / "board.txt"
BOARD_TOLERANCE = 32

DEFINE = re.compile(r"^#define\s+(EDGE_POLL_BOUND|FRAME_BOUND)\s+(\d+)\s*$", re.M)
BAUD = re.compile(r"^// checker: BAUD (\d+) DIV (\d+) FRAG (\d+)\s*$", re.M)
CASE = re.compile(r"^// checker: CASE (8N1|8E1|8N2)((?:\s+0x[0-9a-fA-F]{2})+)\s*$", re.M)
CLOCK = re.compile(r"^// checker: (APB_HZ|CPU_HZ) (\d+)\s*$", re.M)
EDGE = re.compile(r"edge (\d+) ([01])")
TX = re.compile(r"tx ([0-9a-f]{2})")
RX = re.compile(r"rx ([0-9a-f]{2}|none)")


def program(source: str) -> dict:
    """The bauds, frames and bytes `uart_main.c` names, and the poll window."""
    bounds = {name: int(value) for name, value in DEFINE.findall(source)}
    clocks = {name: int(value) for name, value in CLOCK.findall(source)}
    if set(bounds) != {"EDGE_POLL_BOUND", "FRAME_BOUND"} or set(clocks) != {"APB_HZ", "CPU_HZ"}:
        raise ValueError("uart_main.c does not name the poll bound and both clocks")
    bauds = [(int(baud), int(div), int(frag)) for baud, div, frag in BAUD.findall(source)]
    cases = []
    for cfg, raw in CASE.findall(source):
        cases.append((cfg, [int(part, 16) for part in raw.split()]))
    if not bauds or not cases:
        raise ValueError("uart_main.c names no baud or no case")
    return {
        "bound": bounds["EDGE_POLL_BOUND"],
        "frame_bound": bounds["FRAME_BOUND"],
        "apb_hz": clocks["APB_HZ"],
        "cpu_hz": clocks["CPU_HZ"],
        "bauds": bauds,
        "cases": cases,
    }


def bit_lengths(div: int, frag: int, count: int, apb_hz: int, cpu_hz: int) -> list[int]:
    """CPU cycles of each of `count` bits. The accumulator starts at 0."""
    acc = 0
    lengths = []
    for _ in range(count):
        clocks = div
        acc += frag
        if acc >= 16:
            clocks += 1
            acc -= 16
        if (clocks * cpu_hz) % apb_hz != 0:
            raise ValueError(f"a {clocks}-clock bit is not a whole number of CPU cycles")
        lengths.append(clocks * cpu_hz // apb_hz)
    return lengths


def levels_of(cfg: str, byte: int) -> list[int]:
    """Start, data, an even parity bit for `8E1`, then the stop bit or bits. Idle is high."""
    levels = [0]
    for bit in range(8):
        levels.append((byte >> bit) & 1)
    if cfg == "8E1":
        levels.append(bin(byte).count("1") % 2)
    levels.append(1)
    if cfg == "8N2":
        levels.append(1)
    return levels


def expected_edges(cfg: str, byte: int, div: int, frag: int, apb_hz: int, cpu_hz: int) -> list[tuple[int, int]]:
    """Each TXD change as `(cpu_cycles_from_the_start_bit, level)`."""
    levels = levels_of(cfg, byte)
    lengths = bit_lengths(div, frag, len(levels), apb_hz, cpu_hz)
    at = 0
    previous = 1
    edges = []
    for level, length in zip(levels, lengths, strict=True):
        if level != previous:
            edges.append((at, level))
            previous = level
        at += length
    return edges


def transfers(spec: dict) -> list[dict]:
    """One dict per byte the app sends, in the order it sends them."""
    out = []
    for baud, div, frag in spec["bauds"]:
        for cfg, values in spec["cases"]:
            for byte in values:
                edges = expected_edges(cfg, byte, div, frag, spec["apb_hz"], spec["cpu_hz"])
                out.append({"baud": baud, "cfg": cfg, "byte": byte, "edges": edges})
    return out


def console_lines(rows: list[dict]) -> list[str]:
    text = ""
    for row in rows:
        text += chr(int(row["byte"]))
    if text and not text.endswith("\n"):
        raise ValueError("the console's last line has no newline")
    return text.splitlines()


def split_serial(serial_csv: str) -> tuple[list[dict], list[dict]]:
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    usb, uart = [], []
    for row in rows:
        if row["part"] != "u1":
            raise ValueError(f"serial.csv holds a byte of {row['part']}")
        if row["usart"] == "USB_SERIAL_JTAG":
            usb.append(row)
        elif row["usart"] == "UART0":
            uart.append(row)
        else:
            raise ValueError(f"serial.csv holds usart {row['usart']}")
    return usb, uart


def check_console(lines: list[str], sent: list[dict], bound: int) -> list[tuple[int, int]]:
    """The printed edges. Each lies in `[want, want + bound)` and the bytes match."""
    found = []
    cursor = 0
    printed: list[tuple[int, int]] = []

    def take(prefix: str) -> str:
        nonlocal cursor
        if cursor >= len(lines) or not lines[cursor].startswith(prefix):
            got = lines[cursor] if cursor < len(lines) else "end"
            raise ValueError(f"line {cursor + 1} is {got!r}, not {prefix!r}")
        line = lines[cursor]
        cursor += 1
        return line

    while cursor < len(lines) and not lines[cursor].startswith("baud "):
        cursor += 1
    for baud, group in _groups(sent):
        take(f"baud {baud}")
        for cfg, items in group:
            take(f"cfg {cfg}")
            for item in items:
                tx = take("tx ")
                rx = take("rx ")
                if tx != f"tx {item['byte']:02x}" or rx != f"rx {item['byte']:02x}":
                    raise ValueError(f"{cfg} {item['byte']:02x} printed {tx!r} {rx!r}")
                edges = []
                while cursor < len(lines) and lines[cursor].startswith("edge "):
                    match = EDGE.fullmatch(lines[cursor])
                    if not match:
                        raise ValueError(f"bad edge line {lines[cursor]!r}")
                    edges.append((int(match.group(1)), int(match.group(2))))
                    cursor += 1
                want = item["edges"]
                if [(level) for _, level in edges] != [level for _, level in want] or len(edges) != len(want):
                    raise ValueError(f"{cfg} {item['byte']:02x} edges {edges}, not {want}")
                for (got_at, level), (want_at, _) in zip(edges, want, strict=True):
                    if not want_at <= got_at < want_at + bound:
                        raise ValueError(
                            f"{cfg} {item['byte']:02x} edge {got_at} {level} is outside [{want_at}, {want_at + bound})"
                        )
                printed.extend(edges)
                found.append(item)
    if cursor >= len(lines) or lines[cursor] != "UART DONE":
        raise ValueError("the console does not end the bytes with UART DONE")
    if cursor != len(lines) - 1:
        raise ValueError(f"the console has lines after UART DONE: {lines[cursor + 1 :]}")
    if found != sent:
        raise ValueError("the console bytes are not the source bytes")
    return printed


def _groups(sent: list[dict]) -> list[tuple[int, list[tuple[str, list[dict]]]]]:
    grouped = []
    for item in sent:
        if not grouped or grouped[-1][0] != item["baud"]:
            grouped.append((item["baud"], []))
        cases = grouped[-1][1]
        if not cases or cases[-1][0] != item["cfg"]:
            cases.append((item["cfg"], []))
        cases[-1][1].append(item)
    return grouped


def script_rows(rows: list[dict], usb: list[dict]) -> list[dict]:
    """UART rows at or after the first `baud` line. Earlier rows are the boot log."""
    text = ""
    at = None
    for row in usb:
        if not text:
            at = int(row["t_ps"])
        text += chr(int(row["byte"]))
        if text.endswith("\n"):
            if text.startswith("baud "):
                return [item for item in rows if int(item["t_ps"]) >= at]
            text = ""
    raise ValueError("the console has no baud line")


def check_uart_rows(rows: list[dict], sent: list[dict]) -> None:
    """Each byte is received and then sent, and the two rows share a start time."""
    if len(rows) != 2 * len(sent):
        raise ValueError(f"UART0 has {len(rows)} rows, not {2 * len(sent)}")
    for index, item in enumerate(sent):
        rx, tx = rows[2 * index], rows[2 * index + 1]
        byte = str(item["byte"])
        if rx["byte"] != byte or tx["byte"] != byte:
            raise ValueError(f"UART0 rows {rx['byte']} and {tx['byte']} are not {byte}")
        if rx["t_ps"] != tx["t_ps"]:
            raise ValueError(f"byte {byte} starts at {rx['t_ps']} on RX and {tx['t_ps']} on TX")


def transcript(lines: list[str]) -> list[str]:
    """The app's lines from the first `baud` through `UART DONE`."""
    start = next((i for i, line in enumerate(lines) if line.startswith("baud ")), None)
    if start is None or "UART DONE" not in lines[start:]:
        raise ValueError("the console has no baud line and UART DONE")
    end = lines.index("UART DONE", start)
    return lines[start : end + 1]


def check_board(sim_lines: list[str], board_text: str, tolerance: int = BOARD_TOLERANCE) -> None:
    """The capture's bytes match. Each edge offset is within `tolerance` CPU cycles."""
    board_lines = [line.strip() for line in board_text.splitlines() if line.strip()]
    sim_body = transcript(sim_lines)
    board_body = transcript(board_lines)
    if len(sim_body) != len(board_body):
        raise ValueError(f"the board capture has {len(board_body)} lines, not {len(sim_body)}")
    for index, (sim_line, board_line) in enumerate(zip(sim_body, board_body, strict=True)):
        sim_edge = EDGE.fullmatch(sim_line)
        board_edge = EDGE.fullmatch(board_line)
        if sim_edge and board_edge:
            sim_at, sim_level = int(sim_edge.group(1)), int(sim_edge.group(2))
            board_at, board_level = int(board_edge.group(1)), int(board_edge.group(2))
            if sim_level != board_level or abs(sim_at - board_at) > tolerance:
                raise ValueError(
                    f"edge {index} is {sim_at} {sim_level} in the run and {board_at} {board_level} on the board"
                )
            continue
        if sim_line != board_line:
            raise ValueError(f"line {index + 1} is {sim_line!r} in the run and {board_line!r} on the board")


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


def check(run: Path, board: Path | None = None) -> None:
    spec = program(SOURCE.read_text())
    sent = transfers(spec)
    netlist = json.loads((EXAMPLE / "netlist.json").read_text())
    usb, uart = split_serial((run / "traces" / "serial.csv").read_text())
    lines = console_lines(usb)
    check_console(lines, sent, spec["bound"])
    check_uart_rows(script_rows(uart, usb), sent)
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    board_path = BOARD if board is None else board
    if board_path.is_file():
        check_board(lines, board_path.read_text())
        compared = f", edges within {BOARD_TOLERANCE} cycles of the board"
    else:
        compared = ""
    print(f"esp32s3-uart: {len(sent)} bytes, edges inside {spec['bound']} cycles{compared}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    parser.add_argument("--board", type=Path, default=None)
    args = parser.parse_args()
    try:
        if args.board is None:
            check(args.check)
        else:
            check(args.check, args.board)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"esp32s3-uart: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
