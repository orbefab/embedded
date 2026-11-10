"""What `esp32s3_ring` accepts and what it refuses."""

import json
import shutil
import tempfile
from pathlib import Path

from esp32s3_ring import LINES, RESET_PS, SOURCE, SPAN_PS, check, grb_text, parse_source, payload, short_hash, train


def expect(name: str, ok: bool) -> None:
    assert ok, name


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def _write_run(folder: Path, source: Path, starts: list[int] | None = None) -> None:
    frames, one, zero = parse_source(source.read_text())
    trains = [train(frame, one, zero) for frame in frames]
    fall = 100_000_000_000
    if starts is None:
        starts = [120_000_000_000, 220_000_000_000, 320_000_000_000]
    rows = []
    for start_ps, pairs in zip(starts, trains, strict=True):
        rows.append((start_ps + SPAN_PS, start_ps, payload(pairs)))
    link = ["t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash,lanes"]
    for end, start_ps, data in rows:
        link.append(f"{end},{start_ps},u1,GPIO0,pulses,,idle=0,{len(data)},{short_hash(data)},")
    black_at = fall + RESET_PS
    led_rows = ["t_ps,part,count,grb", f"{black_at},ring,12,{'0' * 72}"]
    for (_end, _start, _data), pairs, frame in zip(rows, trains, frames, strict=True):
        at = _end - pairs[-1][1] + RESET_PS
        led_rows.append(f"{at},ring,12,{grb_text(frame)}")
    vcd = "\n".join(
        [
            "$timescale 1 ps $end",
            "$scope module u1 $end",
            "$var wire 1 ! GPIO0 $end",
            "$upscope $end",
            "$scope module ring $end",
            '$var wire 1 " DIN $end',
            "$upscope $end",
            "$enddefinitions $end",
            "#0",
            "$dumpvars",
            "1!",
            '1"',
            "$end",
            f"#{fall}",
            "0!",
            '0"',
            "",
        ]
    )
    serial = ["t_ps,part,usart,byte"]
    text = "".join(line + "\n" for line in LINES)
    for char in text:
        serial.append(f"0,u1,USB_SERIAL_JTAG,{ord(char)}")
    pixels = [[green, red, blue] for red, green, blue in frames[2]]
    observations = {"leds": {"ring": {"kind": "ws2812", "count": 12, "order": "grb", "pixels": pixels}}}
    netlist = {
        "parts": [
            {
                "kind": "esp32s3",
                "rom": {"sha256": "abc", "bytes": 3},
                "firmware": {"flash": [{"offset": 8, "sha256": "def", "bytes": 2}]},
            }
        ]
    }
    manifest = {
        "blobs": [
            {"role": "rom", "sha256": "abc", "bytes": 3},
            {"role": "flash", "sha256": "def", "bytes": 2, "offset": 8},
        ]
    }
    diagnostics = [{"code": "chip.unmodeled_register", "severity": "warning", "message": "kept"}]
    files = {
        "traces/link.csv": "\n".join(link) + "\n",
        "traces/leds.csv": "\n".join(led_rows) + "\n",
        "traces/logic.vcd": vcd,
        "traces/serial.csv": "\n".join(serial) + "\n",
        "observations.json": json.dumps(observations),
        "netlist.json": json.dumps(netlist),
        "manifest.json": json.dumps(manifest),
        "diagnostics.json": json.dumps(diagnostics),
    }
    for name, body in files.items():
        path = folder / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)


def _drop_row(text: str, index: int) -> str:
    lines = text.splitlines()
    header, body = lines[0], lines[1:]
    del body[index]
    return header + "\n" + "\n".join(body) + "\n"


def _replace_row_time(text: str, index: int, at: int) -> str:
    rows = text.splitlines()
    cells = rows[index + 1].split(",")
    cells[0] = str(at)
    rows[index + 1] = ",".join(cells)
    return "\n".join(rows) + "\n"


def _replace_colour(text: str, index: int) -> str:
    rows = text.splitlines()
    cells = rows[index + 1].split(",")
    cells[3] = "ff" + cells[3][2:]
    rows[index + 1] = ",".join(cells)
    return "\n".join(rows) + "\n"


def _swap_leds(text: str, index: int) -> str:
    rows = text.splitlines()
    cells = rows[index + 1].split(",")
    colour = cells[3]
    cells[3] = colour[6:12] + colour[:6] + colour[12:]
    rows[index + 1] = ",".join(cells)
    return "\n".join(rows) + "\n"


def _replace_hash(text: str) -> str:
    rows = text.splitlines()
    cells = rows[1].split(",")
    cells[8] = ("0" if cells[8][0] != "0" else "1") + cells[8][1:]
    rows[1] = ",".join(cells)
    return "\n".join(rows) + "\n"


def test_frames_and_folder() -> None:

    source = SOURCE.read_text()
    moved = source.replace("frames[FRAMES][LEDS][3]", "frames[LEDS][FRAMES][3]", 1)
    short = source.replace("{0, 0, 48}", "", 1)
    expect("a moved table fails", raises(lambda: parse_source(moved)))
    expect("a short table fails", raises(lambda: parse_source(short)))
    folder = Path(tempfile.mkdtemp(prefix="ring-test-"))
    try:
        _write_run(folder, SOURCE)
        expect("a good folder passes", not raises(lambda: check(folder)))
        good_link = (folder / "traces" / "link.csv").read_text()
        good_leds = (folder / "traces" / "leds.csv").read_text()
        (folder / "traces" / "link.csv").write_text(_replace_hash(good_link))
        expect("a wrong train hash fails", raises(lambda: check(folder)))
        (folder / "traces" / "link.csv").write_text(good_link)
        (folder / "traces" / "leds.csv").write_text(_replace_colour(good_leds, 2))
        expect("a wrong colour fails", raises(lambda: check(folder)))
        late = int(good_leds.splitlines()[2].split(",")[0]) + 1
        (folder / "traces" / "leds.csv").write_text(_replace_row_time(good_leds, 1, late))
        expect("a row 1 ps late fails", raises(lambda: check(folder)))
        (folder / "traces" / "leds.csv").write_text(_drop_row(good_leds, 0))
        expect("a missing black row fails", raises(lambda: check(folder)))
        # 100 µs is shorter than the reset, so frame 0's latch falls after record 1 starts.
        record0 = 120_000_000_000
        record1 = record0 + SPAN_PS + 100_000_000
        record2 = record1 + SPAN_PS + 100_000_000_000
        _write_run(folder, SOURCE, [record0, record1, record2])
        expect("a latch after the next record starts fails", raises(lambda: check(folder)))
        (folder / "traces" / "link.csv").write_text(good_link)
        (folder / "traces" / "leds.csv").write_text(good_leds)
        fifth = good_leds + good_leds.splitlines()[-1] + "\n"
        (folder / "traces" / "leds.csv").write_text(fifth)
        expect("a fifth leds.csv row fails", raises(lambda: check(folder)))
        (folder / "traces" / "leds.csv").write_text(good_leds)
        extra = good_link + good_link.splitlines()[-1] + "\n"
        (folder / "traces" / "link.csv").write_text(extra)
        expect("an extra pulses row fails", raises(lambda: check(folder)))
        (folder / "traces" / "link.csv").write_text(_drop_row(good_link, 1))
        expect("a missing pulses row fails", raises(lambda: check(folder)))
        (folder / "traces" / "link.csv").write_text(good_link)
        (folder / "traces" / "leds.csv").write_text(_swap_leds(good_leds, 1))
        expect("swapped LEDs fail", raises(lambda: check(folder)))
    finally:
        shutil.rmtree(folder)
