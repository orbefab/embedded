# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-uart` apart from the engine.

    uv run packages/engine/scripts/esp32s3_uart.py --check RUN_DIR

The app waits for `g`, then for 115200 and 1000000 baud sends the bytes named in
`uart_main.c`, reads each back through the pad, and prints each edge of GPIO43 as
`edge <offset> <level>`. At 115200 it then writes a back-to-back burst and prints
that burst's edges from the first store, so `clkdiv_frag` adds up past the poll
bound. The offset is the `ccount` after the `GPIO_IN1` load that saw the new level,
minus the `ccount` read in the instruction before the FIFO store. A printed offset
lies in `[edge, edge + EDGE_POLL_BOUND)`.

The bytes and the bit lengths come from that file and from the divider it writes.
`traces/serial.csv` holds the console on `USB_SERIAL_JTAG` and each UART byte, sent and
received, on `UART0`. The run must hold no error, and `manifest.json` must list the ROM,
then each flash region with its offset.

`packages/engine/tests/circuits/esp32s3-uart/board.txt`, when it is present, is a console
capture of the same app. The bytes must match. Each later edge of a frame is compared
by its offset from that frame's start bit, within two polls. The start bit is the CPU's
latency from the `CCOUNT` sample, and the compare leaves it out. The first frame and
the burst's first frame open on the entry into the poll, so their intervals are left
out as well. The span from the burst's first stop to its last edge is compared within
the same two polls. Both of those stamps are taken once the poll is in its loop. A
missing file skips that compare.
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

DEFINE = re.compile(r"^#define\s+(EDGE_POLL_BOUND|FRAME_BOUND)\s+(\d+)\s*$", re.M)
BAUD = re.compile(r"^// checker: BAUD (\d+) DIV (\d+) FRAG (\d+)\s*$", re.M)
CASE = re.compile(r"^// checker: CASE (8N1|8E1|8N2)((?:\s+0x[0-9a-fA-F]{2})+)\s*$", re.M)
BURST = re.compile(r"^// checker: BURST (\d+) (8N1|8E1|8N2) (\d+) 0x([0-9a-fA-F]{2})\s*$", re.M)
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
    bursts = [
        {"baud": int(baud), "cfg": cfg, "count": int(count), "byte": int(byte, 16)}
        for baud, cfg, count, byte in BURST.findall(source)
    ]
    if not bauds or not cases:
        raise ValueError("uart_main.c names no baud or no case")
    known = {baud for baud, _, _ in bauds}
    if any(burst["baud"] not in known for burst in bursts):
        raise ValueError("uart_main.c names a burst at a baud it does not send")
    return {
        "bound": bounds["EDGE_POLL_BOUND"],
        "frame_bound": bounds["FRAME_BOUND"],
        "apb_hz": clocks["APB_HZ"],
        "cpu_hz": clocks["CPU_HZ"],
        "bauds": bauds,
        "cases": cases,
        "bursts": bursts,
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


def edges_in_frame(cfg: str, byte: int) -> int:
    """How many TXD changes one frame of `byte` makes. Idle is high."""
    previous = 1
    count = 0
    for level in levels_of(cfg, byte):
        if level != previous:
            count += 1
            previous = level
    return count


def _stop_edge_index(cfg: str, byte: int) -> int:
    """Index within one frame of the edge that begins the stop bit.

    The bit before the stop is low on the burst byte, so that edge exists.
    A frame whose bit before the stop is already high has none.
    """
    levels = levels_of(cfg, byte)
    stop_at = 10 if cfg == "8E1" else 9
    previous = 1
    index = 0
    for bit, level in enumerate(levels):
        if level == previous:
            continue
        if bit == stop_at:
            return index
        index += 1
        previous = level
    raise ValueError(f"{cfg} byte {byte:02x} has no edge at the stop bit")


def burst_span(edges: list[tuple[int, int]], cfg: str, byte: int) -> int:
    """Cycles from the first frame's stop edge to the last edge of the burst."""
    stop = _stop_edge_index(cfg, byte)
    if not edges or stop >= len(edges):
        raise ValueError(f"the burst has no stop edge among {len(edges)} edges")
    return edges[-1][0] - edges[stop][0]


def board_tolerance(poll: int) -> int:
    """Two polls, the allowance between the run's interval and the board's.

    The poll loop loads `GPIO_IN1`, reads `CCOUNT`, and branches back while the
    level holds. The miss path is that load, `extui`, a `call8` to `rsr.ccount`
    (`entry`, the read, `retw.n`), the frame-bound check, and the branch back.
    On the core's costs that pass is 24 cycles: one per instruction, two more on
    a taken branch, and one more when a taken branch's next instruction crosses
    a word. `poll` is `EDGE_POLL_BOUND`. The probe publishes it as the ceiling
    on the pass, and a stamp is the `CCOUNT` after the load that saw the edge,
    so the stamp lies in `[edge, edge + poll)`.

    An interval is the difference of two such stamps. Each is late by less than
    one poll, so one run's interval is off by less than one poll. The model and
    the board each quantise, and the allowance is two polls.
    """
    return 2 * poll


def expected_edges(cfg: str, byte: int, div: int, frag: int, apb_hz: int, cpu_hz: int) -> list[tuple[int, int]]:
    """Each TXD change as `(cpu_cycles_from_the_start_bit, level)`."""
    return frame_edges(cfg, byte, 1, div, frag, apb_hz, cpu_hz)


def frame_edges(
    cfg: str, byte: int, count: int, div: int, frag: int, apb_hz: int, cpu_hz: int
) -> list[tuple[int, int]]:
    """Edges of `count` back-to-back frames, from the first start bit.

    Each frame's fractional accumulator starts at 0, as the shifter does. The
    extras still add, which is what puts a later edge past `EDGE_POLL_BOUND`
    of the same edge with `frag` 0.
    """
    edges = []
    at = 0
    previous = 1
    for _ in range(count):
        levels = levels_of(cfg, byte)
        lengths = bit_lengths(div, frag, len(levels), apb_hz, cpu_hz)
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
        for burst in spec["bursts"]:
            if burst["baud"] != baud:
                continue
            edges = frame_edges(burst["cfg"], burst["byte"], burst["count"], div, frag, spec["apb_hz"], spec["cpu_hz"])
            for index in range(burst["count"]):
                out.append(
                    {
                        "baud": baud,
                        "cfg": burst["cfg"],
                        "byte": burst["byte"],
                        "edges": edges if index == 0 else [],
                        "burst": True,
                    }
                )
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
    plain = [item for item in sent if not item.get("burst")]
    for baud, group in _groups(plain):
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
        burst_items = [item for item in sent if item.get("burst") and item["baud"] == baud]
        if burst_items:
            take("burst")
            edges = []
            while cursor < len(lines) and lines[cursor].startswith("edge "):
                match = EDGE.fullmatch(lines[cursor])
                if not match:
                    raise ValueError(f"bad edge line {lines[cursor]!r}")
                edges.append((int(match.group(1)), int(match.group(2))))
                cursor += 1
            want = burst_items[0]["edges"]
            if [level for _, level in edges] != [level for _, level in want] or len(edges) != len(want):
                raise ValueError(f"burst edges {edges}, not {want}")
            for (got_at, level), (want_at, _) in zip(edges, want, strict=True):
                if not want_at <= got_at < want_at + bound:
                    raise ValueError(f"burst edge {got_at} {level} is outside [{want_at}, {want_at + bound})")
            printed.extend(edges)
            for item in burst_items:
                tx = take("tx ")
                rx = take("rx ")
                if tx != f"tx {item['byte']:02x}" or rx != f"rx {item['byte']:02x}":
                    raise ValueError(f"burst printed {tx!r} {rx!r}")
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
    """UART rows after the boot log, before the first `baud` line's USB packet finishes.

    The app prints `baud 115200` and immediately starts the first UART frame, so at Full-Speed USB
    the `baud` packet finishes slightly after that frame starts. Rows after the preceding console
    line exclude the boot log while keeping the first frame.
    """
    text = ""
    at = None
    prev_at = None
    for row in usb:
        if not text:
            at = int(row["t_ps"])
        text += chr(int(row["byte"]))
        if text.endswith("\n"):
            if text.startswith("baud "):
                if prev_at is not None:
                    return [item for item in rows if int(item["t_ps"]) > prev_at]
                return [item for item in rows if int(item["t_ps"]) >= at]
            prev_at = at
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


def _take_edges(lines: list[str], start: int) -> tuple[list[tuple[int, int]], int]:
    edges = []
    cursor = start
    while cursor < len(lines) and lines[cursor].startswith("edge "):
        match = EDGE.fullmatch(lines[cursor])
        if not match:
            raise ValueError(f"bad edge line {lines[cursor]!r}")
        edges.append((int(match.group(1)), int(match.group(2))))
        cursor += 1
    return edges, cursor


def _compare_frames(
    sim: list[tuple[int, int]],
    board: list[tuple[int, int]],
    per_frame: int,
    tolerance: int,
    where: str,
    entry_frames: int,
) -> None:
    """Later edges by their offset from the frame's start bit. The start bit is left out.

    The first `entry_frames` frames open on the entry into the poll. That start bit is
    late by the time to reach the loop, and the later edges of the frame are not, so
    their interval is that latency rather than the UART. Those frames are checked for
    length and level only.
    """
    if len(sim) != len(board):
        raise ValueError(f"{where} has {len(sim)} edges in the run and {len(board)} on the board")
    if per_frame <= 0 or len(sim) % per_frame != 0:
        raise ValueError(f"{where} has {len(sim)} edges, not frames of {per_frame}")
    for frame_index, frame_at in enumerate(range(0, len(sim), per_frame)):
        sim_frame = sim[frame_at : frame_at + per_frame]
        board_frame = board[frame_at : frame_at + per_frame]
        if sim_frame[0][1] != 0 or board_frame[0][1] != 0:
            raise ValueError(f"{where} does not open with the start bit")
        sim_origin, board_origin = sim_frame[0][0], board_frame[0][0]
        for index, ((sim_at, sim_level), (board_at, board_level)) in enumerate(
            zip(sim_frame, board_frame, strict=True)
        ):
            if sim_level != board_level:
                raise ValueError(f"{where} edge {index} is level {sim_level} in the run and {board_level} on the board")
            if frame_index < entry_frames or index == 0:
                continue
            sim_rel, board_rel = sim_at - sim_origin, board_at - board_origin
            if abs(sim_rel - board_rel) >= tolerance:
                raise ValueError(
                    f"{where} frame {frame_index} edge {index} is {sim_rel} cycles after the start"
                    f" in the run and {board_rel} on the board, tolerance {tolerance}"
                )


def check_board(sim_lines: list[str], board_text: str, spec: dict, tolerance: int) -> None:
    """The capture's bytes match. Each later edge agrees, from its frame's start bit, within `tolerance`.

    The burst's span, from its first stop to its last edge, agrees within `tolerance` too.

    The first edge of a frame is the start bit. Its offset is the time from the
    `CCOUNT` sample before the FIFO store to the poll that sees the pin fall.
    That is CPU latency, instruction timing and misses in the 80 MHz instruction
    cache, which the model does not claim to the cycle. The timers check allows
    4000 cycles for several such misses, and 4000 is wider than one bit at
    1 Mbaud, 240 CPU cycles, so a bound of that shape would hide a missing bit.
    A bound under 240 cannot cover the cold start on a board capture on
    2026-11-18, 936 cycles, nor the burst's first edge, 612 cycles. The compare
    leaves each frame's first edge out.

    An interval measured from the start bit still contains that bit's latency.
    It cancels when the later edges are late by the same poll. It does not when
    the start is the entry into the loop and the later edges are the loop already
    running. The first `send_byte` is that cold entry: on the same capture its
    stop is 90 cycles after the model's, so the interval is short by 846, the
    miss. The burst's first edge is the entry into that other poll, and that
    frame's stop is 97 cycles after the model's, so the interval is short by 515.
    A tolerance wide enough for either is wider than one bit at 1 Mbaud, so those
    two frames are not interval-compared. Their bytes, edge counts and levels
    still have to match. Later frames of the burst are later passes of the same
    poll, and they are compared. Every other interval on that capture is within
    40 cycles.

    One bit at the 115200 divider is 2082 CPU cycles and one at 1 Mbaud is 240,
    and `tolerance` is two polls, 128 when `EDGE_POLL_BOUND` is 64, so a missing
    bit fails. A divider one count off adds one APB clock, 3 CPU cycles, to
    every bit. The ninth edge of a frame moves 27 cycles, inside the allowance.
    From the burst's first start bit the same error moves the last edge 237
    cycles. The compare uses each frame's own start, so that sum is not what it
    holds.

    The burst's first stop is the first stamp after the poll is in its loop. On
    the 2026-11-18 capture that stop is 97 cycles after the model's, and the
    first start is 612, so the interval between them is the entry. The last edge
    is a later pass of the same loop. The span from the first stop to the last
    edge subtracts two stamps of that loop, and the allowance is the same two
    polls. The span is the seven frames between those edges. This burst's frame
    is ten bits, so a divider one count off lengthens the span by 7 * 10 * 3,
    210 CPU cycles, past the allowance. On that capture the span is 4 cycles
    short of the model's.

    A fraction one count off adds one long bit in a ten-bit frame. `frag` 7
    makes four long bits and `frag` 8 makes five, and one frame's edges move by
    at most that one extra clock, 3 CPU cycles. The long bits add across the
    burst. The last edge is seven full frames plus the nine bits to the eighth
    stop. Each of the seven frames contributes one extra clock, and the ninth
    bit is long at `frag` 8 and short at `frag` 7, which is one clock more.
    Eight clocks are 24 CPU cycles from the burst's first start bit. The span
    drops the shared nine-bit prefix, so it moves by the seven frames only, 21
    CPU cycles. 24 is inside the 64-cycle console window, and 3 and 21 are
    inside the board allowance, so a fraction one count off passes both checks.
    Neither the 64-cycle console window nor the board compare resolves a
    fraction error of one count.
    """
    board_lines = [line.strip() for line in board_text.splitlines() if line.strip()]
    sim_body = transcript(sim_lines)
    board_body = transcript(board_lines)
    if len(sim_body) != len(board_body):
        raise ValueError(f"the board capture has {len(board_body)} lines, not {len(sim_body)}")
    cfg = ""
    byte = None
    previous = ""
    cursor = 0
    cold = True
    while cursor < len(sim_body):
        sim_line, board_line = sim_body[cursor], board_body[cursor]
        if sim_line.startswith("edge ") or board_line.startswith("edge "):
            sim_edges, sim_end = _take_edges(sim_body, cursor)
            board_edges, board_end = _take_edges(board_body, cursor)
            if sim_end != board_end:
                raise ValueError(f"{previous or 'edges'} has edges through different lines on the board")
            if previous == "burst":
                burst = spec["bursts"][0]
                per_frame = edges_in_frame(burst["cfg"], burst["byte"])
                where = "burst"
                entry_frames = 1
            elif cfg and byte is not None:
                per_frame = edges_in_frame(cfg, byte)
                where = f"{cfg} {byte:02x}"
                entry_frames = 1 if cold else 0
                cold = False
            else:
                raise ValueError("edges come before a frame")
            _compare_frames(sim_edges, board_edges, per_frame, tolerance, where, entry_frames)
            if previous == "burst":
                sim_span = burst_span(sim_edges, burst["cfg"], burst["byte"])
                board_span = burst_span(board_edges, burst["cfg"], burst["byte"])
                if abs(sim_span - board_span) >= tolerance:
                    raise ValueError(
                        f"the burst spans {sim_span} cycles from its first stop to its last edge"
                        f" in the run and {board_span} on the board, tolerance {tolerance}"
                    )
            cursor = sim_end
            continue
        if sim_line != board_line:
            raise ValueError(f"line {cursor + 1} is {sim_line!r} in the run and {board_line!r} on the board")
        if sim_line.startswith("cfg "):
            cfg = sim_line.split()[1]
        elif sim_line.startswith("tx "):
            byte = int(sim_line.split()[1], 16)
        previous = sim_line.split()[0]
        cursor += 1


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
        tolerance = board_tolerance(spec["bound"])
        check_board(lines, board_path.read_text(), spec, tolerance)
        compared = f", frame intervals and the burst span within {tolerance} cycles of the board"
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
