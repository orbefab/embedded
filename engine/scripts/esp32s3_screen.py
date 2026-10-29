# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `examples/esp32s3-screen` against the board's capture, apart from the engine.

    uv run scripts/esp32s3_screen.py --check RUN_DIR
    uv run scripts/esp32s3_screen.py --self-test

`examples/esp32s3-screen/board.txt` holds the lines the board printed for the example's app as it
drew: every `P`, `C`, `F`, `S` and `B` line of `firmware/esp32s3/idf/screen/main/wrap.c`, in order.
`P` is a panel register write with its parameters, `C` a colour write with its length and hash,
`B` a colour write's bytes, `F` a flush with its rectangle, and `S` each SPI transaction a
transfer took, with its flags and length in bits. The last field of a `P`, `C` or `F` line is the
board's time in microseconds, which no check reads.

From those lines alone, every transfer is the 32-bit command word on one lane, then its bytes in
the transactions its `S` lines give, with CS held until the last. Each row of `traces/link.csv`
must be one of those transactions, in order, with its CS state, no phases, its byte count and
hash, and its data lanes. Each row of `traces/display.csv` must be one flush, in order, with its
rectangle, byte count and hash. `traces/display-lcd1.rgb565` must be the frame memory the flushes
leave, with every pixel no flush reached zero, and the console in `traces/serial.csv` must hold
the board's lines with their times left out. The run must hold no error, no `chip.protocol` and no
`chip.link_miswired`.
"""

import argparse
import base64
import csv
import hashlib
import io
import json
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BOARD = ROOT / "examples" / "esp32s3-screen" / "board.txt"
PANEL = 360
QIO = 1 << 1
CS_KEEP = 1 << 8
TIMED = ("P", "C", "F")
FAILING = ("chip.protocol", "chip.link_miswired")
LINK = ["part", "pin", "kind", "cs", "phases", "bytes", "hash", "lanes"]
DISPLAY = ["part", "x0", "y0", "x1", "y1", "bytes", "hash"]


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
                transfers.append({"seq": int(f[1]), "cmd": int(f[2], 16), "data": data, "hash": h16(data),
                                  "length": int(f[3]), "qio": False, "txns": []})
            elif f[0] == "C":
                transfers.append({"seq": int(f[1]), "cmd": int(f[2], 16), "data": b"", "hash": f[4],
                                  "length": int(f[3]), "qio": True, "txns": []})
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
        if caset["data"] != bytes([x0 >> 8, x0 & 255, x1 >> 8, x1 & 255]) or \
                raset["data"] != bytes([y0 >> 8, y0 & 255, y1 >> 8, y1 & 255]):
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
                data = t["data"][at:at + bits // 8]
                at += bits // 8
            rows.append(("u1", "GPIO12", "spi", "held" if flags & CS_KEEP else "released", "", str(len(data)), h16(data),
                         "4" if flags & QIO else "1"))
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
            memory[at:at + width] = fl["data"][row * width:(row + 1) * width]
    return bytes(memory)


def console_lines(serial_csv: str) -> list[str]:
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    if any(row["part"] != "u1" or row["usart"] != "USB_SERIAL_JTAG" for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG")
    text = bytes(int(row["byte"]) for row in rows).decode("ascii")
    return [line for line in text.replace("\r\n", "\n").split("\n") if line[:2] in ("P ", "C ", "F ", "S ", "B ")]


def compare(name: str, got: list, want: list) -> None:
    if got != want:
        at = next((i for i, (a, b) in enumerate(zip(got, want)) if a != b), min(len(got), len(want)))
        seen = got[at] if at < len(got) else "nothing"
        wanted = want[at] if at < len(want) else "nothing"
        raise ValueError(f"{name} row {at + 1} is {seen}, the board's is {wanted} ({len(got)} rows, the board's {len(want)})")


def table(path: Path, columns: list[str]) -> list[tuple]:
    rows = list(csv.DictReader(io.StringIO(path.read_text())))
    return [tuple(row[c] for c in columns) for row in rows]


def check(run: Path) -> str:
    board = BOARD.read_text()
    transfers, flushes = parse(board)
    traces = run / "traces"
    compare("link.csv", table(traces / "link.csv", LINK), link_rows(transfers))
    compare("display.csv", table(traces / "display.csv", DISPLAY), display_rows(flushes))
    want = frame(flushes)
    got = (traces / "display-lcd1.rgb565").read_bytes()
    if got != want:
        raise ValueError(f"display-lcd1.rgb565 has SHA-256 {hashlib.sha256(got).hexdigest()}, the board's frame "
                         f"{hashlib.sha256(want).hexdigest()}")
    compare("the console", untimed(console_lines((traces / "serial.csv").read_text())), untimed(board.splitlines()))
    failing = [d["code"] for d in json.loads((run / "diagnostics.json").read_text())
               if d["severity"] == "error" or d["code"] in FAILING]
    if failing:
        raise ValueError(f"the run holds {failing}")
    return (f"esp32s3-screen: {len(link_rows(transfers))} transactions and {len(flushes)} flushes as the board sent "
            f"them, the board's final frame {hashlib.sha256(want).hexdigest()}, and its console lines")


def write_run(run: Path, link: list[tuple], display: list[tuple], image: bytes, console: str,
              diagnostics: list[dict]) -> None:
    """A run folder with those rows, frame, console text and diagnostics, as `sim run` lays it out."""
    traces = run / "traces"
    traces.mkdir(parents=True, exist_ok=True)
    (traces / "link.csv").write_text("t_ps,start_ps," + ",".join(LINK) + "\n"
                                     + "".join(f"{k + 1},{k},{','.join(row)}\n" for k, row in enumerate(link)))
    (traces / "display.csv").write_text("t_ps," + ",".join(DISPLAY) + "\n"
                                        + "".join(f"{k + 1},{','.join(row)}\n" for k, row in enumerate(display)))
    (traces / "display-lcd1.rgb565").write_bytes(image)
    (traces / "serial.csv").write_text("t_ps,part,usart,byte\n"
                                       + "".join(f"{k + 1},u1,USB_SERIAL_JTAG,{b}\n" for k, b in enumerate(console.encode())))
    (run / "diagnostics.json").write_text(json.dumps(diagnostics))


def check_cases() -> list[tuple[str, bool]]:
    """Each case of `check` on a run folder built from the committed capture, and whether it passed."""
    board = BOARD.read_text()
    transfers, flushes = parse(board)
    link, display, image = link_rows(transfers), display_rows(flushes), frame(flushes)
    console = "I (31) boot: screen\r\n" + "".join(line + "\r\n" for line in board.splitlines())
    warning = {"code": "chip.unmodeled_register", "severity": "warning"}
    moved = (*display[3][:1], str(int(display[3][1]) + 1), *display[3][2:])
    flipped = bytearray(image)
    flipped[len(image) // 2] ^= 1
    runs = {
        "the board's run": (link, display, image, console, [warning]),
        "a zeroed frame": (link, display, bytes(len(image)), console, []),
        "a changed frame": (link, display, bytes(flipped), console, []),
        "an error": (link, display, image, console, [{"code": "chip.halted", "severity": "error"}]),
        "a protocol warning": (link, display, image, console, [{"code": "chip.protocol", "severity": "warning"}]),
        "a miswire warning": (link, display, image, console, [{"code": "chip.link_miswired", "severity": "warning"}]),
        "a missing transfer": (link[:-1], display, image, console, []),
        "an extra transfer": ([*link, link[-1]], display, image, console, []),
        "reordered transfers": ([link[1], link[0], *link[2:]], display, image, console, []),
        "a transaction on other lanes": ([(*link[0][:7], "4"), *link[1:]], display, image, console, []),
        "a transaction with phases": ([(*link[0][:4], "8/1:02", *link[0][5:]), *link[1:]], display, image, console, []),
        "a missing flush": (link, display[:-1], image, console, []),
        "an extra flush": (link, [*display, display[-1]], image, console, []),
        "reordered flushes": (link, [display[1], display[0], *display[2:]], image, console, []),
        "a wrong rectangle": (link, [*display[:3], moved, *display[4:]], image, console, []),
        "a short console": (link, display, image, console.rsplit("\r\n", 2)[0] + "\r\n", []),
    }
    results = []
    with tempfile.TemporaryDirectory() as scratch:
        for name, run in runs.items():
            folder = Path(scratch) / str(len(results))
            write_run(folder, *run)
            try:
                check(folder)
                results.append((name, True))
            except ValueError:
                results.append((name, False))
    return results


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

    pixels = bytes(range(8))
    capture = "\n".join([
        "P 1 02002a00 4 00010002 10",
        "S 1 poll 0100 - 0 - 0 32",
        "S 1 poll 0000 - 0 - 0 32",
        "F 2 1 3 2 4 8 " + h16(pixels) + " 11",
        "P 3 02002a00 4 00010002 12",
        "S 3 poll 0100 - 0 - 0 32",
        "S 3 poll 0000 - 0 - 0 32",
        "P 4 02002b00 4 00030004 13",
        "S 4 poll 0100 - 0 - 0 32",
        "S 4 poll 0000 - 0 - 0 32",
        "C 5 32002c00 8 " + h16(pixels) + " 14",
        "S 5 queue 0100 - 0 - 0 32",
        "S 5 queue 0102 - 0 - 0 32",
        "B 5 " + base64.b64encode(pixels[:4]).decode(),
        "S 5 queue 0002 - 0 - 0 32",
        "B 5 " + base64.b64encode(pixels[4:]).decode(),
    ])
    transfers, flushes = parse(capture)
    rows = link_rows(transfers)
    expect("a capture parses", len(transfers) == 4 and len(flushes) == 1)
    expect("the command word is its own held row on one lane",
           rows[0][3:] == ("held", "", "4", h16(bytes.fromhex("02002a00")), "1"))
    expect("a transfer's bytes split as its transactions do",
           [r[3] for r in rows[-3:]] == ["held", "held", "released"] and rows[-1][6] == h16(pixels[4:]))
    expect("colour goes on four lanes", [r[7] for r in rows[-3:]] == ["1", "4", "4"])
    expect("a flush is its display row", display_rows(flushes) == [("lcd1", "1", "3", "2", "4", "8", h16(pixels))])
    painted = bytearray(PANEL * PANEL * 2)
    painted[2 * (3 * PANEL + 1):2 * (3 * PANEL + 3)] = pixels[:4]
    painted[2 * (4 * PANEL + 1):2 * (4 * PANEL + 3)] = pixels[4:]
    expect("a flush paints its rectangle and leaves the rest zero", frame(flushes) == painted)
    expect("times are left out", untimed(["P 1 02 1 00 99", "S 1 poll 0100 - 0 - 0 32"]) == ["P 1 02 1 00", "S 1 poll 0100 - 0 - 0 32"])
    expect("the same rows pass", not raises(lambda: compare("link.csv", rows, link_rows(transfers))))
    expect("a changed hash fails", raises(lambda: compare("link.csv", [*rows[:-1], (*rows[-1][:6], "0" * 16, "4")], rows)))
    expect("a lost row fails", raises(lambda: compare("link.csv", rows[:-1], rows)))
    expect("swapped rows fail", raises(lambda: compare("link.csv", [rows[1], rows[0], *rows[2:]], rows)))
    expect("a released command word fails",
           raises(lambda: compare("link.csv", [(*rows[0][:3], "released", *rows[0][4:]), *rows[1:]], rows)))
    expect("a command word sent as phases fails",
           raises(lambda: compare("link.csv", [(*rows[0][:4], "8+24", *rows[0][5:]), *rows[1:]], rows)))
    expect("bytes that differ from their hash fail", raises(lambda: parse(capture.replace("B 5 AAECAw==", "B 5 AAECBA=="))))
    expect("a colour write on one lane fails", raises(lambda: parse(capture.replace("S 5 queue 0102", "S 5 queue 0100"))))
    expect("CS released mid transfer fails", raises(lambda: parse(capture.replace("S 4 poll 0100", "S 4 poll 0000"))))
    expect("a flush that is not its CASET fails", raises(lambda: parse(capture.replace("F 2 1 3 2 4", "F 2 0 3 2 4"))))
    expect("a gap in the numbering fails", raises(lambda: parse(capture.replace("P 1 ", "P 0 ").replace("S 1 ", "S 0 "))))
    expect("a stray line fails", raises(lambda: parse(capture + "\nX 9")))
    serial = "t_ps,part,usart,byte\n" + "".join(f"1,u1,USB_SERIAL_JTAG,{b}\n" for b in b"I (5) x\r\nP 1 02 1 00 7\r\n")
    expect("the console keeps the transfer lines", console_lines(serial) == ["P 1 02 1 00 7"])
    expect("another port fails", raises(lambda: console_lines(serial.replace("USB_SERIAL_JTAG", "USART0", 1))))
    board_transfers, board_flushes = parse(BOARD.read_text())
    expect("the committed capture holds 263 transfers, 529 transactions and 25 flushes",
           (len(board_transfers), len(link_rows(board_transfers)), len(board_flushes)) == (263, 529, 25))
    for name, passed in check_cases():
        good = name == "the board's run"
        expect(f"check on {name} {'passes' if good else 'fails'}", passed == good)
    expect("the committed capture's final frame is the board's",
           hashlib.sha256(frame(board_flushes)).hexdigest()
           == "882493174920b5371c1e0013545197195abee6055ad2288c35c716aedbc51dfd")
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
        print(check(args.check))
    except (ValueError, OSError, KeyError) as error:
        print(f"esp32s3-screen: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
