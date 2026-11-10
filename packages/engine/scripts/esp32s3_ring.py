# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-ring` against the app's frame table, apart from the engine.

    uv run packages/engine/scripts/esp32s3_ring.py --check RUN_DIR

The frame table and the two symbol shapes are read from
`packages/engine/tests/firmware/esp32s3/idf/ring/main/ring_main.c`. Each frame is encoded here, green then red then
blue, most significant bit first, at 100 ns a tick. `traces/link.csv` must hold that train.
The same trains are decoded with windows written below, and `traces/leds.csv` must show black
and then the three frames. The run folder is the only other input.
"""

import argparse
import csv
import hashlib
import io
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "packages/engine/tests/firmware" / "esp32s3" / "idf" / "ring" / "main" / "ring_main.c"
TICK_PS = 100_000
SPAN_PS = 345_600_000
PAYLOAD_BYTES = 5_184
RESET_PS = 280_000_001
# Inclusive, written here and not taken from the engine.
T0H = (220_000, 380_000)
T1H = (580_000, 1_000_000)
LINES = ["ring ready", "frame 0", "frame 1", "frame 2", "ring done"]
SHAPE = (3, 12, 3)

DECL = re.compile(
    r"static const uint8_t frames\[(\w+)\]\[(\w+)\]\[(\w+)\]\s*=\s*\{",
)
DEFINE = re.compile(r"^#define\s+(\w+)\s+(\d+)\s*$", re.M)
DURATION0 = re.compile(r"\.duration0\s*=\s*one\s*\?\s*(\d+)\s*:\s*(\d+)")
DURATION1 = re.compile(r"\.duration1\s*=\s*one\s*\?\s*(\d+)\s*:\s*(\d+)")


def _dim(token: str, defines: dict[str, int]) -> int:
    if token.isdigit():
        return int(token)
    if token not in defines:
        raise ValueError(f"frames uses {token}, which is not a #define")
    return defines[token]


def _initializer(text: str, open_at: int) -> str:
    depth = 0
    for index in range(open_at, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[open_at : index + 1]
    raise ValueError("the frames initializer does not close")


def _nested(body: str) -> list:
    """A brace tree of integers. A `{` starts a list and a number is a leaf."""
    tokens = re.findall(r"\d+|[{}]", body)
    stack: list[list] = [[]]
    for token in tokens:
        if token == "{":
            stack.append([])
        elif token == "}":
            if len(stack) == 1:
                raise ValueError("the frames initializer has an extra }")
            done = stack.pop()
            stack[-1].append(done)
        else:
            stack[-1].append(int(token))
    if len(stack) != 1 or len(stack[0]) != 1:
        raise ValueError("the frames initializer is not one table")
    return stack[0][0]


def _shape_of(table) -> tuple:
    if not isinstance(table, list) or not table:
        raise ValueError("the frames table is empty")
    if all(isinstance(item, int) for item in table):
        return (len(table),)
    shapes = {_shape_of(item) for item in table}
    if len(shapes) != 1:
        raise ValueError(f"the frames table has mixed shapes {sorted(shapes)}")
    return (len(table), *shapes.pop())


def parse_source(text: str) -> tuple[list, tuple[int, int], tuple[int, int]]:
    """The frame table as red, green, blue, and the 1-bit and 0-bit tick pairs."""
    defines = {name: int(value) for name, value in DEFINE.findall(text)}
    found = list(DECL.finditer(text))
    if len(found) != 1:
        raise ValueError(f"ring_main.c has {len(found)} frames tables, not 1")
    decl = found[0]
    declared = tuple(_dim(token, defines) for token in decl.groups())
    if declared != SHAPE:
        raise ValueError(f"the frames declaration is {list(declared)}, not {list(SHAPE)}")
    table = _nested(_initializer(text, decl.end() - 1))
    found_shape = _shape_of(table)
    if found_shape != SHAPE:
        raise ValueError(f"the frames initializer has shape {list(found_shape)}, not {list(SHAPE)}")
    one = DURATION0.findall(text)
    zero = DURATION1.findall(text)
    if len(one) != 1 or len(zero) != 1:
        raise ValueError("ring_main.c does not name one duration0 and one duration1")
    high1, high0 = int(one[0][0]), int(one[0][1])
    low1, low0 = int(zero[0][0]), int(zero[0][1])
    if (high1, low1) != (8, 4) or (high0, low0) != (3, 9):
        raise ValueError(f"the symbols are 1={high1}/{low1} and 0={high0}/{low0}, not 8/4 and 3/9")
    return table, (high1, low1), (high0, low0)


def train(rgb: list, one: tuple[int, int], zero: tuple[int, int]) -> list[tuple[int, int]]:
    """`(level, ps)` pairs. Channels go out green, red, blue, each bit high first."""
    pairs = []
    for red, green, blue in rgb:
        for channel in (green, red, blue):
            for shift in range(7, -1, -1):
                high, low = one if channel >> shift & 1 else zero
                pairs.append((1, high * TICK_PS))
                pairs.append((0, low * TICK_PS))
    return pairs


def payload(pairs: list[tuple[int, int]]) -> bytes:
    out = bytearray()
    for level, ps in pairs:
        out.append(level)
        out += ps.to_bytes(8, "little")
    return bytes(out)


def short_hash(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()[:16]


def decode(pairs: list[tuple[int, int]]) -> list[list[int]]:
    """Red, green, blue per LED, from each high's width alone."""
    if len(pairs) % 2:
        raise ValueError("the train has an odd number of pulses")
    bits = []
    for index in range(0, len(pairs), 2):
        level, high = pairs[index]
        low_level, _low = pairs[index + 1]
        if level != 1 or low_level != 0:
            raise ValueError("a bit is not a high followed by a low")
        if T0H[0] <= high <= T0H[1]:
            bits.append(0)
        elif T1H[0] <= high <= T1H[1]:
            bits.append(1)
        else:
            raise ValueError(f"a high of {high} ps is outside both windows")
    if len(bits) != SHAPE[1] * 24:
        raise ValueError(f"the train has {len(bits)} bits, not {SHAPE[1] * 24}")
    colours = []
    for led in range(SHAPE[1]):
        channels = []
        for channel in range(3):
            value = 0
            for bit in bits[led * 24 + channel * 8 : led * 24 + (channel + 1) * 8]:
                value = (value << 1) | bit
            channels.append(value)
        green, red, blue = channels
        colours.append([red, green, blue])
    return colours


def grb_text(rgb: list) -> str:
    return "".join(f"{green:02x}{red:02x}{blue:02x}" for red, green, blue in rgb)


def console_lines(serial_csv: str) -> list[str]:
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    if any(row["part"] != "u1" or row["usart"] != "USB_SERIAL_JTAG" for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG")
    text = "".join(chr(int(row["byte"])) for row in rows)
    lines = []
    for line in text.split("\n"):
        if line.endswith("\r"):
            line = line[:-1]
        if line or lines:
            lines.append(line)
    if lines and lines[-1] == "" and text.endswith("\n"):
        lines.pop()
    return lines


def falls(vcd: str) -> dict[str, int]:
    """The one fall time of `u1` `GPIO0` and of `ring` `DIN`, in picoseconds."""
    want = {("u1", "GPIO0"): None, ("ring", "DIN"): None}
    ids: dict[str, tuple[str, str]] = {}
    scope: list[str] = []
    for line in vcd.splitlines():
        if line.startswith("$scope"):
            parts = line.split()
            scope.append(parts[2] if len(parts) > 2 else "")
        elif line.startswith("$upscope"):
            if scope:
                scope.pop()
        elif line.startswith("$var"):
            parts = line.split()
            if len(parts) >= 6 and tuple(scope[-1:] + [parts[4]]) in want:
                ids[parts[3]] = (scope[-1], parts[4])
        elif line.startswith("$dumpvars") or line.startswith("#") or line[:1] in "01":
            break
    if set(ids.values()) != set(want):
        raise ValueError(f"logic.vcd names {sorted(ids.values())}, not GPIO0 and ring.DIN")
    level = {key: None for key in want}
    fallen = {key: [] for key in want}
    risen = {key: [] for key in want}
    now = 0
    body = False
    for line in vcd.splitlines():
        if line.startswith("#"):
            now = int(line[1:])
            body = True
            continue
        if not body or not line or line[0] not in "01":
            continue
        key = ids.get(line[1:])
        if key is None:
            continue
        bit = line[0]
        previous = level[key]
        level[key] = bit
        if previous == "1" and bit == "0":
            fallen[key].append(now)
        elif previous == "0" and bit == "1":
            risen[key].append(now)
    out = {}
    for key in want:
        if risen[key] or level[key] != "0" or len(fallen[key]) != 1:
            raise ValueError(f"{key[0]}.{key[1]} falls {fallen[key]} and rises {risen[key]}, not once")
        out[f"{key[0]}.{key[1]}"] = fallen[key][0]
    if out["u1.GPIO0"] != out["ring.DIN"]:
        raise ValueError(f"GPIO0 and ring.DIN fall at {out}, not together")
    return out


def check_manifest(manifest: dict, netlist: dict) -> None:
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    want = [{"role": "rom", **part["rom"]}]
    want += [
        {"role": "flash", "sha256": region["sha256"], "bytes": region["bytes"], "offset": region["offset"]}
        for region in part["firmware"]["flash"]
    ]
    if manifest["blobs"] != want:
        raise ValueError(f"manifest.json lists {manifest['blobs']}, not {want}")


def check(run: Path, source: Path = SOURCE) -> None:
    frames, one, zero = parse_source(source.read_text())
    trains = [train(frame, one, zero) for frame in frames]
    for index, (frame, pairs) in enumerate(zip(frames, trains, strict=True)):
        if decode(pairs) != frame:
            raise ValueError(f"frame {index} does not decode to its table row")
    link = list(csv.DictReader((run / "traces" / "link.csv").read_text().splitlines()))
    if len(link) != 3 or any(row["kind"] != "pulses" for row in link):
        raise ValueError(f"link.csv has {len(link)} rows, not 3 pulses rows")
    hashes = []
    for index, (row, pairs) in enumerate(zip(link, trains, strict=True)):
        data = payload(pairs)
        digest = short_hash(data)
        hashes.append(digest)
        if (
            row["part"] != "u1"
            or row["pin"] != "GPIO0"
            or row["phases"] != "idle=0"
            or int(row["bytes"]) != PAYLOAD_BYTES
            or len(data) != PAYLOAD_BYTES
            or row["hash"] != digest
            or int(row["t_ps"]) - int(row["start_ps"]) != SPAN_PS
        ):
            raise ValueError(
                f"pulses row {index} is {row['part']} {row['pin']} {row['phases']} "
                f"{row['bytes']} {row['hash']} span {int(row['t_ps']) - int(row['start_ps'])}, "
                f"not u1 GPIO0 idle=0 {PAYLOAD_BYTES} {digest} span {SPAN_PS}"
            )
    fall = falls((run / "traces" / "logic.vcd").read_text())["u1.GPIO0"]
    leds = list(csv.DictReader((run / "traces" / "leds.csv").read_text().splitlines()))
    if len(leds) != 4:
        raise ValueError(f"leds.csv has {len(leds)} rows, not 4")
    black = "0" * (SHAPE[1] * 6)
    expected = [black, *(grb_text(frame) for frame in frames)]
    times = [fall + RESET_PS]
    for row, pairs in zip(link, trains, strict=True):
        if pairs[-1][0] != 0:
            raise ValueError("a frame does not end in a low")
        times.append(int(row["t_ps"]) - pairs[-1][1] + RESET_PS)
    starts = [int(row["start_ps"]) for row in link]
    for index, (row, colour, at) in enumerate(zip(leds, expected, times, strict=True)):
        if row["part"] != "ring" or int(row["count"]) != SHAPE[1] or row["grb"] != colour:
            raise ValueError(f"leds row {index} is {row['grb']}, not {colour}")
        if int(row["t_ps"]) != at:
            raise ValueError(f"leds row {index} is at {row['t_ps']} ps, not {at}")
        if index < 3 and int(row["t_ps"]) >= starts[index]:
            raise ValueError(f"leds row {index} at {row['t_ps']} ps is not before record {index} at {starts[index]}")
    observations = json.loads((run / "observations.json").read_text())
    chain = observations["leds"]["ring"]
    pixels = [[green, red, blue] for red, green, blue in frames[2]]
    if chain.get("kind") != "ws2812" or chain.get("count") != SHAPE[1] or chain.get("order") != "grb":
        raise ValueError(f"observations leds.ring is {chain.get('kind')} {chain.get('count')} {chain.get('order')}")
    if chain.get("pixels") != pixels:
        raise ValueError("observations leds.ring is not frame 2")
    lines = console_lines((run / "traces" / "serial.csv").read_text())
    try:
        start = lines.index("ring ready")
        end = lines.index("ring done", start)
    except ValueError:
        raise ValueError("the console never prints ring ready and ring done") from None
    got = lines[start : end + 1]
    if got != LINES:
        raise ValueError(f"the console from ring ready to ring done is {got}, not {LINES}")
    diagnostics = json.loads((run / "diagnostics.json").read_text())
    for item in diagnostics:
        if item.get("code") != "chip.unmodeled_register" or item.get("severity") == "error":
            raise ValueError(f"the run holds {item.get('severity')} {item.get('code')}")
        if "RMT" in json.dumps(item):
            raise ValueError("a diagnostic names RMT")
    check_manifest(
        json.loads((run / "manifest.json").read_text()),
        json.loads((run / "netlist.json").read_text()),
    )
    print(f"esp32s3-ring: 4 led rows, 3 pulses rows, hashes {', '.join(hashes)}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", type=Path, metavar="RUN", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except ValueError as error:
        print(f"esp32s3-ring: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
