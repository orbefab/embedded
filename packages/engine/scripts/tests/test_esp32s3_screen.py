"""What `esp32s3_screen` accepts and what it refuses."""

import base64
import hashlib
import json
import tempfile
from pathlib import Path

from esp32s3_screen import (
    BOARD,
    DISPLAY,
    DUTIES,
    LINK,
    PANEL,
    check,
    compare,
    console_lines,
    display_rows,
    frame,
    h16,
    link_rows,
    parse,
    untimed,
)


def expect(name: str, ok: bool) -> None:
    assert ok, name


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def write_run(
    run: Path, link: list[tuple], display: list[tuple], image: bytes, console: str, diagnostics: list[dict]
) -> None:
    """A run folder with those rows, frame, console text and diagnostics, as `orbe-run run` lays it out."""
    traces = run / "traces"
    traces.mkdir(parents=True, exist_ok=True)
    (traces / "link.csv").write_text(
        "t_ps,start_ps,"
        + ",".join(LINK)
        + "\n"
        + "".join(f"{k + 1},{k},{','.join(row)}\n" for k, row in enumerate(link))
    )
    (traces / "display.csv").write_text(
        "t_ps," + ",".join(DISPLAY) + "\n" + "".join(f"{k + 1},{','.join(row)}\n" for k, row in enumerate(display))
    )
    (traces / "display-lcd1.rgb565").write_bytes(image)
    (traces / "serial.csv").write_text(
        "t_ps,part,usart,byte\n" + "".join(f"{k + 1},u1,USB_SERIAL_JTAG,{b}\n" for k, b in enumerate(console.encode()))
    )
    (run / "diagnostics.json").write_text(json.dumps(diagnostics))


def check_cases() -> list[tuple[str, bool]]:
    """Each case of `check` on a run folder built from the committed capture, and whether it passed."""
    board = BOARD.read_text()
    transfers, flushes = parse(board)
    display, image = display_rows(flushes), frame(flushes)
    link = [DUTIES[0], *link_rows(transfers)[:4], DUTIES[1], *link_rows(transfers)[4:]]
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
        "a missing duty": ([row for row in link if row != DUTIES[0]], display, image, console, []),
        "another duty": ([*link, (*DUTIES[1][:4], "2048/4096@5000", "", "", "")], display, image, console, []),
        "reordered duties": ([DUTIES[1], *link[1:5], DUTIES[0], *link[6:]], display, image, console, []),
        "a duty on another pin": ([(DUTIES[0][0], "GPIO20", *DUTIES[0][2:]), *link[1:]], display, image, console, []),
        "a missing transfer": (link[:-1], display, image, console, []),
        "an extra transfer": ([*link, link[-1]], display, image, console, []),
        "reordered transfers": ([link[0], link[2], link[1], *link[3:]], display, image, console, []),
        "a transaction on other lanes": ([link[0], (*link[1][:7], "4"), *link[2:]], display, image, console, []),
        "a transaction with phases": (
            [link[0], (*link[1][:4], "8/1:02", *link[1][5:]), *link[2:]],
            display,
            image,
            console,
            [],
        ),
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


def test_capture_and_run_folder() -> None:

    pixels = bytes(range(8))
    capture = "\n".join(
        [
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
        ]
    )
    transfers, flushes = parse(capture)
    rows = link_rows(transfers)
    expect("a capture parses", len(transfers) == 4 and len(flushes) == 1)
    expect(
        "the command word is its own held row on one lane",
        rows[0][3:] == ("held", "", "4", h16(bytes.fromhex("02002a00")), "1"),
    )
    expect(
        "a transfer's bytes split as its transactions do",
        [r[3] for r in rows[-3:]] == ["held", "held", "released"] and rows[-1][6] == h16(pixels[4:]),
    )
    expect("colour goes on four lanes", [r[7] for r in rows[-3:]] == ["1", "4", "4"])
    expect("a flush is its display row", display_rows(flushes) == [("lcd1", "1", "3", "2", "4", "8", h16(pixels))])
    painted = bytearray(PANEL * PANEL * 2)
    painted[2 * (3 * PANEL + 1) : 2 * (3 * PANEL + 3)] = pixels[:4]
    painted[2 * (4 * PANEL + 1) : 2 * (4 * PANEL + 3)] = pixels[4:]
    expect("a flush paints its rectangle and leaves the rest zero", frame(flushes) == painted)
    expect(
        "times are left out",
        untimed(["P 1 02 1 00 99", "S 1 poll 0100 - 0 - 0 32"]) == ["P 1 02 1 00", "S 1 poll 0100 - 0 - 0 32"],
    )
    expect("the same rows pass", not raises(lambda: compare("link.csv", rows, link_rows(transfers))))
    expect(
        "a changed hash fails", raises(lambda: compare("link.csv", [*rows[:-1], (*rows[-1][:6], "0" * 16, "4")], rows))
    )
    expect("a lost row fails", raises(lambda: compare("link.csv", rows[:-1], rows)))
    expect("swapped rows fail", raises(lambda: compare("link.csv", [rows[1], rows[0], *rows[2:]], rows)))
    expect(
        "a released command word fails",
        raises(lambda: compare("link.csv", [(*rows[0][:3], "released", *rows[0][4:]), *rows[1:]], rows)),
    )
    expect(
        "a command word sent as phases fails",
        raises(lambda: compare("link.csv", [(*rows[0][:4], "8+24", *rows[0][5:]), *rows[1:]], rows)),
    )
    expect(
        "bytes that differ from their hash fail", raises(lambda: parse(capture.replace("B 5 AAECAw==", "B 5 AAECBA==")))
    )
    expect(
        "a colour write on one lane fails", raises(lambda: parse(capture.replace("S 5 queue 0102", "S 5 queue 0100")))
    )
    expect("CS released mid transfer fails", raises(lambda: parse(capture.replace("S 4 poll 0100", "S 4 poll 0000"))))
    expect("a flush that is not its CASET fails", raises(lambda: parse(capture.replace("F 2 1 3 2 4", "F 2 0 3 2 4"))))
    expect(
        "a gap in the numbering fails", raises(lambda: parse(capture.replace("P 1 ", "P 0 ").replace("S 1 ", "S 0 ")))
    )
    expect("a stray line fails", raises(lambda: parse(capture + "\nX 9")))
    serial = "t_ps,part,usart,byte\n" + "".join(f"1,u1,USB_SERIAL_JTAG,{b}\n" for b in b"I (5) x\r\nP 1 02 1 00 7\r\n")
    expect("the console keeps the transfer lines", console_lines(serial) == ["P 1 02 1 00 7"])
    expect("another port fails", raises(lambda: console_lines(serial.replace("USB_SERIAL_JTAG", "USART0", 1))))
    board_transfers, board_flushes = parse(BOARD.read_text())
    expect(
        "the committed capture holds 263 transfers, 529 transactions and 25 flushes",
        (len(board_transfers), len(link_rows(board_transfers)), len(board_flushes)) == (263, 529, 25),
    )
    for name, passed in check_cases():
        good = name == "the board's run"
        expect(f"check on {name} {'passes' if good else 'fails'}", passed == good)
    expect(
        "the committed capture's final frame is the board's",
        hashlib.sha256(frame(board_flushes)).hexdigest()
        == "882493174920b5371c1e0013545197195abee6055ad2288c35c716aedbc51dfd",
    )
