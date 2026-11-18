# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-screen` against the board's capture, apart from the engine.

    uv run packages/engine/scripts/esp32s3_screen.py --check RUN_DIR

`packages/engine/tests/circuits/esp32s3-screen/board.txt` holds the lines the board printed for the example's app as it
drew: every `P`, `C`, `F`, `S` and `B` line of
`packages/engine/tests/firmware/esp32s3/idf/screen/main/wrap.c`, in order.
`P` is a panel register write with its parameters, `C` a colour write with its length and hash,
`B` a colour write's bytes, `F` a flush with its rectangle, and `S` each SPI transaction a
transfer took, with its flags and length in bits. The last field of a `P`, `C` or `F` line is the
board's time in microseconds, which no check reads.

From those lines alone, every transfer is the 32-bit command word on one lane, then its bytes in
the transactions its `S` lines give, with CS held until the last. Each `spi` row of
`traces/link.csv` must be one of those transactions, in order, with its CS state, no phases, its
byte count and hash, and its data lanes. Its `duty` rows must be the backlight's two duties on
GPIO21 that the app's source sets, 0 when it configures the channel and then 4095 of 4096 steps,
both at 5 kHz. Each row of `traces/display.csv` must be one flush, in order, with its rectangle,
byte count and hash. `traces/display-lcd1.rgb565` must be the frame memory the flushes leave, with
every pixel no flush reached zero, and the console in `traces/serial.csv` must hold the board's
lines with their times left out. The run must hold no error, no `chip.protocol` and no
`chip.link_miswired`.
"""

import argparse
import base64
import csv
import hashlib
import io
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BOARD = ROOT / "packages/engine/tests/circuits" / "esp32s3-screen" / "board.txt"
PANEL = 360
QIO = 1 << 1
CS_KEEP = 1 << 8
TIMED = ("P", "C", "F")
FAILING = ("chip.protocol", "chip.link_miswired")
LINK = ["part", "pin", "kind", "cs", "phases", "bytes", "hash", "lanes"]
DISPLAY = ["part", "x0", "y0", "x1", "y1", "bytes", "hash"]
DUTIES = [("u1", "GPIO21", "duty", "", f"{duty}/4096@5000", "", "", "") for duty in (0, 4095)]


def h16(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()[:16]


def untimed(lines: list[str]) -> list[str]:
    return [line.rsplit(" ", 1)[0] if line.split(" ", 1)[0] in TIMED else line for line in lines]


def parse(text: str) -> tuple[list[dict], list[dict]]:
    """The transfers and the flushes of a capture, each checked against its own bytes."""
    transfers, flushes = [], []
    for n, line in enumerate(text.splitlines(), 1):
        f = line.split(" ")
        try:
            if f[0] == "P":
                data = bytes.fromhex(f[4]) if int(f[3]) else b""
                transfers.append(
                    {
                        "seq": int(f[1]),
                        "cmd": int(f[2], 16),
                        "data": data,
                        "hash": h16(data),
                        "length": int(f[3]),
                        "qio": False,
                        "txns": [],
                    }
                )
            elif f[0] == "C":
                transfers.append(
                    {
                        "seq": int(f[1]),
                        "cmd": int(f[2], 16),
                        "data": b"",
                        "hash": f[4],
                        "length": int(f[3]),
                        "qio": True,
                        "txns": [],
                    }
                )
            elif f[0] == "B":
                transfers[-1]["data"] += base64.b64decode(f[2], validate=True)
            elif f[0] == "S":
                transfers[-1]["txns"].append((int(f[3], 16), int(f[8])))
            elif f[0] == "F":
                flushes.append({"seq": int(f[1]), "rect": tuple(map(int, f[2:6])), "bytes": int(f[6]), "hash": f[7]})
            else:
                raise ValueError(f"unknown line kind {f[0]!r}")
            if f[0] in ("B", "S") and int(f[1]) != transfers[-1]["seq"]:
                raise ValueError(f"an {f[0]} line of transfer {f[1]} after transfer {transfers[-1]['seq']}")
        except (IndexError, ValueError) as error:
            raise ValueError(f"board.txt line {n}: {error}") from None
    for t in transfers:
        if h16(t["data"]) != t["hash"] or len(t["data"]) != t["length"]:
            raise ValueError(f"transfer {t['seq']}: its bytes do not give its length and hash")
        if not t["txns"] or t["txns"][0][1] != 32 or t["txns"][0][0] & QIO:
            raise ValueError(f"transfer {t['seq']}: its first transaction is not the command word on one lane")
        if sum(bits for _, bits in t["txns"][1:]) != 8 * len(t["data"]):
            raise ValueError(f"transfer {t['seq']}: its transactions do not carry its bytes")
        for i, (flags, _) in enumerate(t["txns"]):
            if bool(flags & CS_KEEP) == (i == len(t["txns"]) - 1) or (i and bool(flags & QIO) != t["qio"]):
                raise ValueError(f"transfer {t['seq']}: transaction {i} has flags {flags:#x}")
    by_seq = {t["seq"]: t for t in transfers}
    if sorted([*by_seq, *(fl["seq"] for fl in flushes)]) != list(range(1, len(transfers) + len(flushes) + 1)):
        raise ValueError("the transfers and flushes are not numbered 1 to n")
    for fl in flushes:
        x0, y0, x1, y1 = fl["rect"]
        caset, raset, colour = (by_seq.get(fl["seq"] + k) for k in (1, 2, 3))
        if not (caset and raset and colour and colour["qio"]):
            raise ValueError(f"flush {fl['seq']}: not followed by CASET, RASET and a colour write")
        if caset["data"] != bytes([x0 >> 8, x0 & 255, x1 >> 8, x1 & 255]) or raset["data"] != bytes(
            [y0 >> 8, y0 & 255, y1 >> 8, y1 & 255]
        ):
            raise ValueError(f"flush {fl['seq']}: CASET or RASET does not give its rectangle")
        if (colour["hash"], len(colour["data"])) != (fl["hash"], fl["bytes"]):
            raise ValueError(f"flush {fl['seq']}: its colour write has other bytes")
        fl["data"] = colour["data"]
    return transfers, flushes


def link_rows(transfers: list[dict]) -> list[tuple]:
    rows = []
    for t in transfers:
        at = 0
        for i, (flags, bits) in enumerate(t["txns"]):
            if i == 0:
                data = t["cmd"].to_bytes(4, "big")
            else:
                data = t["data"][at : at + bits // 8]
                at += bits // 8
            rows.append(
                (
                    "u1",
                    "GPIO12",
                    "spi",
                    "held" if flags & CS_KEEP else "released",
                    "",
                    str(len(data)),
                    h16(data),
                    "4" if flags & QIO else "1",
                )
            )
    return rows


def display_rows(flushes: list[dict]) -> list[tuple]:
    return [("lcd1", *map(str, fl["rect"]), str(fl["bytes"]), fl["hash"]) for fl in flushes]


def frame(flushes: list[dict]) -> bytes:
    memory = bytearray(PANEL * PANEL * 2)
    for fl in flushes:
        x0, y0, x1, y1 = fl["rect"]
        width = 2 * (x1 - x0 + 1)
        for row in range(y1 - y0 + 1):
            at = 2 * ((y0 + row) * PANEL + x0)
            memory[at : at + width] = fl["data"][row * width : (row + 1) * width]
    return bytes(memory)


def console_lines(serial_csv: str) -> list[str]:
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    known = {"USB_SERIAL_JTAG", "UART0", "UART1", "UART2"}
    if any(row["part"] != "u1" or row["usart"] not in known for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG or a UART")
    rows = [row for row in rows if row["usart"] == "USB_SERIAL_JTAG"]
    text = bytes(int(row["byte"]) for row in rows).decode("ascii")
    return [line for line in text.replace("\r\n", "\n").split("\n") if line[:2] in ("P ", "C ", "F ", "S ", "B ")]


def compare(name: str, got: list, want: list) -> None:
    if got != want:
        at = next((i for i, (a, b) in enumerate(zip(got, want, strict=False)) if a != b), min(len(got), len(want)))
        seen = got[at] if at < len(got) else "nothing"
        wanted = want[at] if at < len(want) else "nothing"
        raise ValueError(
            f"{name} row {at + 1} is {seen}, the board's is {wanted} ({len(got)} rows, the board's {len(want)})"
        )


def table(path: Path, columns: list[str]) -> list[tuple]:
    rows = list(csv.DictReader(io.StringIO(path.read_text())))
    return [tuple(row[c] for c in columns) for row in rows]


def check(run: Path) -> str:
    board = BOARD.read_text()
    transfers, flushes = parse(board)
    traces = run / "traces"
    link = table(traces / "link.csv", LINK)
    compare("link.csv's spi", [row for row in link if row[2] != "duty"], link_rows(transfers))
    compare("link.csv's duty", [row for row in link if row[2] == "duty"], DUTIES)
    compare("display.csv", table(traces / "display.csv", DISPLAY), display_rows(flushes))
    want = frame(flushes)
    got = (traces / "display-lcd1.rgb565").read_bytes()
    if got != want:
        raise ValueError(
            f"display-lcd1.rgb565 has SHA-256 {hashlib.sha256(got).hexdigest()}, the board's frame "
            f"{hashlib.sha256(want).hexdigest()}"
        )
    compare("the console", untimed(console_lines((traces / "serial.csv").read_text())), untimed(board.splitlines()))
    failing = [
        d["code"]
        for d in json.loads((run / "diagnostics.json").read_text())
        if d["severity"] == "error" or d["code"] in FAILING
    ]
    if failing:
        raise ValueError(f"the run holds {failing}")
    return (
        f"esp32s3-screen: {len(link_rows(transfers))} transactions and {len(flushes)} flushes as the board sent "
        f"them, the backlight's duties, the board's final frame {hashlib.sha256(want).hexdigest()}, "
        "and its console lines"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        print(check(args.check))
    except (ValueError, OSError, KeyError) as error:
        print(f"esp32s3-screen: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
