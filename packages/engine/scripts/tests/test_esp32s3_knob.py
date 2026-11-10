"""What `esp32s3_knob` accepts and what it refuses."""

import base64
import io
import json
import sys
import tempfile
from pathlib import Path

import esp32s3_knob
from esp32s3_knob import (
    AFTER_PS,
    CLOSE_FROM_PS,
    CLOSE_LEN_PS,
    DEFAULT_START,
    MIN_GAP_PS,
    READY_BEFORE_PS,
    TOUCH_APART_PS,
    TOUCH_HOLD_PS,
    check,
    events,
    h16,
    load_refs,
    main,
    marks,
    panel,
    script,
    write_refs,
)


def expect(name: str, fn, refuse: bool = False) -> None:
    try:
        fn()
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        if not refuse:
            raise AssertionError(f"{name}: {error}") from error
        return
    if refuse:
        raise AssertionError(f"{name}: accepted")


def serial_of(text: str, ready_at: int = 1_000_000_000_000) -> str:
    ready_nl = text.find("\n", text.find("KNOB READY")) if "KNOB READY" in text else -1
    rows = ["t_ps,part,usart,byte"]
    for index, char in enumerate(text):
        stamp = ready_at if index == ready_nl else index + 1
        rows.append(f"{stamp},u1,USB_SERIAL_JTAG,{ord(char)}")
    return "\n".join(rows) + "\n"


def display_csv(flushes: list[dict]) -> str:
    rows = ["t_ps,part,x0,y0,x1,y1,bytes,hash"]
    for index, fl in enumerate(flushes, 1):
        rows.append(f"{index},lcd1,{fl['x0']},{fl['y0']},{fl['x1']},{fl['y1']},{fl['length']},{fl['hash']}")
    return "\n".join(rows) + "\n"


def link_csv(second: str = "4096/4096@5000") -> str:
    rows = [
        "t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash,lanes",
        "1,1,u1,GPIO21,duty,,0/4096@5000,,,",
        f"2,2,u1,GPIO21,duty,,{second},,,",
    ]
    return "\n".join(rows) + "\n"


def synthetic() -> str:
    pixels = bytes(range(16))
    digest = h16(pixels)
    caset = bytes([0x00, 0x00, 0x00, 0x03])
    raset = bytes([0x00, 0x00, 0x00, 0x01])
    lines = [
        "BEFORE THE START",
        "KNOB START",
        "KNOB INFO sample",
        "KNOB FRAME 0 BEGIN",
        f"F 1 0 0 3 1 16 {digest} 10",
        f"P 2 02002a00 4 {caset.hex()} 11",
        "S 2 poll 0100 - 0 - 0 32",
        "S 2 poll 0000 - 0 - 0 32",
        f"P 3 02002b00 4 {raset.hex()} 12",
        "S 3 poll 0100 - 0 - 0 32",
        "S 3 poll 0000 - 0 - 0 32",
        f"C 4 32002c00 16 {digest} 13",
        "B 4 " + base64.b64encode(pixels[:8]).decode(),
        "B 4 " + base64.b64encode(pixels[8:]).decode(),
        "S 4 poll 0100 - 0 - 0 32",
        "S 4 queue 0102 - 0 - 0 64",
        "S 4 queue 0002 - 0 - 0 64",
        "KNOB FRAME 0 END",
        "K a=1 b=0",
        "T down 1",
        "T up",
        "KNOB READY",
        "KNOB DONE",
    ]
    return "\n".join(lines) + "\n"


def write_run(
    root: Path,
    text: str,
    refs: dict,
    ready_at: int = 1_000_000_000_000,
    duty: str = "4096/4096@5000",
    diagnostic: str = "chip.unmodeled_register",
    frame: bytes | None = None,
) -> None:
    traces = root / "traces"
    traces.mkdir(parents=True, exist_ok=True)
    (traces / "serial.csv").write_text(serial_of(text, ready_at))
    (traces / "display.csv").write_text(display_csv(refs["flushes"]))
    image = frame if frame is not None else base64.b64decode(refs["frame_b64"])
    (traces / "display-lcd1.rgb565").write_bytes(image)
    (traces / "link.csv").write_text(link_csv(duty))
    (root / "diagnostics.json").write_text(json.dumps([{"code": diagnostic, "severity": "warning"}]))


def bad(got: object):
    raise ValueError(got)


def _write(path: Path, text: str) -> Path:
    path.write_text(text)
    return path


def test_captures_refs_and_netlist() -> None:

    text = synthetic()
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        refs_path = root / "refs.json"
        write_refs(
            [_write(root / "a.txt", text), _write(root / "b.txt", text), _write(root / "c.txt", text)],
            refs_path,
            DEFAULT_START,
        )
        refs = load_refs(refs_path)
        expect(
            "the line before the start is not a mark",
            lambda: "BEFORE THE START" not in json.dumps(refs) or bad("present"),
        )
        ready_at = 1_000_000_000_000
        run = root / "run"
        write_run(run, text, refs, ready_at)

        def check_summary() -> None:
            summary = check(run, refs_path)
            if f"ready at {ready_at} ps" not in summary:
                bad(summary)

        expect("panel", lambda: panel(run, refs_path, False))
        expect("marks", lambda: marks(run, refs_path))
        expect("events", lambda: events(run, refs_path))
        expect("check", check_summary)

        shaped = json.loads(refs_path.read_text())
        shaped["transfers"][2]["hash"] = "0" * 16
        shaped["flushes"][0]["hash"] = "0" * 16
        shaped["frame_b64"] = base64.b64encode(b"\x00" * len(base64.b64decode(shaped["frame_b64"]))).decode()
        shaped_path = root / "shaped.json"
        shaped_path.write_text(json.dumps(shaped))
        expect("a colour hash fails panel", lambda: panel(run, shaped_path, False), refuse=True)
        expect("panel --shape drops the colour hash and the frame", lambda: panel(run, shaped_path, True))

        moved = json.loads(refs_path.read_text())
        moved["flushes"][0]["x1"] = 4
        moved_path = root / "moved.json"
        moved_path.write_text(json.dumps(moved))
        expect("panel --shape keeps the rectangle", lambda: panel(run, moved_path, True), refuse=True)

        param = json.loads(refs_path.read_text())
        param["transfers"][1]["hash"] = "0" * 16
        param_path = root / "param.json"
        param_path.write_text(json.dumps(param))
        expect("panel --shape keeps a parameter hash", lambda: panel(run, param_path, True), refuse=True)

        other_marks = text.replace("KNOB INFO sample", "KNOB INFO other")
        marked = root / "marked"
        write_run(marked, other_marks, refs)
        expect("panel reads no mark", lambda: panel(marked, refs_path, False))
        expect("a changed mark fails", lambda: marks(marked, refs_path), refuse=True)

        other_events = text.replace("K a=1 b=0", "K a=1 b=1")
        evented = root / "evented"
        write_run(evented, other_events, refs)
        expect("a changed K line fails events", lambda: events(evented, refs_path), refuse=True)
        expect("events reads no mark", lambda: events(marked, refs_path))

        late = root / "late"
        write_run(late, text, refs, ready_at=READY_BEFORE_PS)
        expect("KNOB READY at 2.0 s fails", lambda: check(late, refs_path), refuse=True)
        early = root / "early"
        write_run(early, text, refs, ready_at=READY_BEFORE_PS - 1)
        expect("KNOB READY 1 ps earlier passes", lambda: check(early, refs_path))

        warned = root / "warned"
        write_run(warned, text, refs, diagnostic="chip.clock")
        expect("another diagnostic fails", lambda: check(warned, refs_path), refuse=True)

        dim = root / "dim"
        write_run(dim, text, refs, duty="4095/4096@5000")
        expect("duty 4095 of 4096 fails", lambda: check(dim, refs_path), refuse=True)

        short = text.replace("S 2 poll 0000 - 0 - 0 32", "S 2 poll 0100 - 0 - 0 32")
        expect(
            "CS held on the last transaction fails",
            lambda: write_refs(
                [
                    _write(root / "bad-1.txt", short),
                    _write(root / "bad-2.txt", short),
                    _write(root / "bad-3.txt", short),
                ],
                root / "bad.json",
                DEFAULT_START,
            ),
            refuse=True,
        )

        drifted = text.replace("KNOB INFO sample", "KNOB INFO drifted")
        expect(
            "captures that disagree fail",
            lambda: write_refs(
                [_write(root / "d1.txt", text), _write(root / "d2.txt", text), _write(root / "d3.txt", drifted)],
                root / "drift.json",
                DEFAULT_START,
            ),
            refuse=True,
        )

        net_path = root / "net.json"
        net_path.write_text(
            json.dumps(
                {
                    "analysis": {"transient": {"stop_ps": 1, "max_step_ps": 1, "grid_ps": 1}},
                    "parts": [
                        {"name": "u1", "kind": "esp32s3", "console": [{"t_ps": 100_000_000_000, "bytes": [103]}]},
                        {
                            "name": "sw_a",
                            "kind": "switch",
                            "initial": "open",
                            "events": [{"t_ps": 1, "state": "closed"}],
                        },
                        {"name": "sw_b", "kind": "switch", "initial": "open", "events": []},
                        {"name": "u2", "kind": "cst816", "touches": []},
                    ],
                }
            )
        )
        script(refs_path, net_path)
        net = json.loads(net_path.read_text())
        parts = {part["name"]: part for part in net["parts"]}
        expect(
            "one closing of a is 80 ms from 2.0 s",
            lambda: (
                parts["sw_a"]["events"]
                == [
                    {"t_ps": CLOSE_FROM_PS, "state": "closed"},
                    {"t_ps": CLOSE_FROM_PS + CLOSE_LEN_PS, "state": "open"},
                ]
                or bad(parts["sw_a"]["events"])
            ),
        )
        expect("b has no closing", lambda: parts["sw_b"]["events"] == [] or bad("b"))
        touch_at = CLOSE_FROM_PS + CLOSE_LEN_PS + TOUCH_APART_PS
        expect(
            "dot 1 is held 300 ms",
            lambda: (
                parts["u2"]["touches"]
                == [
                    {"t_ps": touch_at, "x": 180, "y": 80, "down": True},
                    {"t_ps": touch_at + TOUCH_HOLD_PS, "x": 180, "y": 80, "down": False},
                ]
                or bad(parts["u2"]["touches"])
            ),
        )
        x_ps = touch_at + AFTER_PS
        core = next(part for part in net["parts"] if part["kind"] == "esp32s3")
        expect(
            "g stays and x follows the touch by 500 ms",
            lambda: (
                core["console"]
                == [
                    {"t_ps": 100_000_000_000, "bytes": [103]},
                    {"t_ps": x_ps, "bytes": [120]},
                ]
                or bad(core["console"])
            ),
        )
        expect(
            "the stop moves to 500 ms after x",
            lambda: (
                net["analysis"]["transient"]["stop_ps"] == x_ps + AFTER_PS
                or bad(net["analysis"]["transient"]["stop_ps"])
            ),
        )

        empty = dict(refs)
        empty["events"] = []
        empty_path = root / "empty.json"
        empty_path.write_text(json.dumps(empty))
        empty_net = root / "empty-net.json"
        empty_net.write_text(net_path.read_text())
        bare = json.loads(empty_net.read_text())
        bare["analysis"]["transient"]["stop_ps"] = 1
        bare["parts"][0]["console"] = [{"t_ps": 100_000_000_000, "bytes": [103]}]
        empty_net.write_text(json.dumps(bare))
        script(empty_path, empty_net)
        got = json.loads(empty_net.read_text())
        core = next(part for part in got["parts"] if part["kind"] == "esp32s3")
        expect(
            "no event puts x at 2.5 s",
            lambda: core["console"][1]["t_ps"] == CLOSE_FROM_PS + AFTER_PS or bad(core["console"]),
        )

        jumped = dict(refs)
        jumped["events"] = ["K a=2 b=0"]
        jump_path = root / "jump.json"
        jump_path.write_text(json.dumps(jumped))
        expect("a count that skips fails", lambda: script(jump_path, net_path), refuse=True)

        hit = root / "hit.json"
        hit.write_text(
            json.dumps(
                {
                    "analysis": {"transient": {"stop_ps": 1, "max_step_ps": 1, "grid_ps": 1}},
                    "parts": [
                        {
                            "name": "u1",
                            "kind": "esp32s3",
                            "console": [{"t_ps": CLOSE_FROM_PS + AFTER_PS, "bytes": [103]}],
                        },
                        {"name": "sw_a", "kind": "switch", "initial": "open", "events": []},
                        {"name": "sw_b", "kind": "switch", "initial": "open", "events": []},
                        {"name": "u2", "kind": "cst816", "touches": []},
                    ],
                }
            )
        )
        expect("console bytes under 2000 ps apart fail", lambda: script(empty_path, hit), refuse=True)

        def three(name: str, body: str, dest: str):
            paths = []
            for index in range(3):
                path = root / f"{name}-{index}.txt"
                path.write_text(body)
                paths.append(path)
            return write_refs(paths, root / dest, DEFAULT_START)

        k_only = text.replace("K a=1 b=0", "K a=2 b=0")
        expect(
            "captures that differ only in a K line fail",
            lambda: write_refs(
                [_write(root / "k1.txt", text), _write(root / "k2.txt", text), _write(root / "k3.txt", k_only)],
                root / "k.json",
                DEFAULT_START,
            ),
            refuse=True,
        )
        t_only = text.replace("T down 1", "T down 2")
        expect(
            "captures that differ only in a T line fail",
            lambda: write_refs(
                [_write(root / "t1.txt", text), _write(root / "t2.txt", text), _write(root / "t3.txt", t_only)],
                root / "t.json",
                DEFAULT_START,
            ),
            refuse=True,
        )

        def extra_p(payload: bytes) -> str:
            block = f"P 5 00000000 4 {payload.hex()} 20\nS 5 poll 0100 - 0 - 0 32\nS 5 poll 0000 - 0 - 0 32\n"
            return text.replace("KNOB FRAME 0 END\n", block + "KNOB FRAME 0 END\n")

        expect(
            "captures that differ only in a transfer hash fail",
            lambda: write_refs(
                [
                    _write(root / "h1.txt", extra_p(b"\x01\x02\x03\x04")),
                    _write(root / "h2.txt", extra_p(b"\x01\x02\x03\x04")),
                    _write(root / "h3.txt", extra_p(b"\x05\x06\x07\x08")),
                ],
                root / "h.json",
                DEFAULT_START,
            ),
            refuse=True,
        )

        csv_run = root / "csv-run"
        write_run(csv_run, text, refs)
        csv_path = csv_run / "traces" / "display.csv"
        csv_path.write_text(csv_path.read_text().replace("lcd1,0,0,3,1,16,", "lcd1,0,0,4,1,16,", 1))
        expect("a display.csv row that differs fails", lambda: panel(csv_run, refs_path, False), refuse=True)

        frame_run = root / "frame-run"
        image = base64.b64decode(refs["frame_b64"])
        write_run(frame_run, text, refs, frame=bytes([image[0] ^ 0xFF]) + image[1:])
        expect("a frame that differs fails", lambda: panel(frame_run, refs_path, False), refuse=True)

        other_rect = text.replace("P 2 02002a00 4 00000003 ", "P 2 02002a00 4 00000004 ", 1)
        expect(
            "a flush whose CASET gives another rectangle fails",
            lambda: three("rect", other_rect, "rect.json"),
            refuse=True,
        )
        short_bits = text.replace("S 4 queue 0002 - 0 - 0 64", "S 4 queue 0002 - 0 - 0 32", 1)
        expect(
            "a transaction that does not carry the bytes fails",
            lambda: three("bits", short_bits, "bits.json"),
            refuse=True,
        )
        orphan = (
            "C 5 00000000 4 " + ("0" * 16) + " 20\n"
            "B 5 " + base64.b64encode(b"abcd").decode() + "\n"
            "S 5 poll 0100 - 0 - 0 32\n"
            "S 5 queue 0002 - 0 - 0 32\n"
        )
        bad_hash = text.replace("KNOB FRAME 0 END\n", orphan + "KNOB FRAME 0 END\n")
        expect(
            "a transfer whose bytes do not give its hash fails",
            lambda: three("bhash", bad_hash, "bhash.json"),
            refuse=True,
        )
        qio_first = text.replace("S 2 poll 0100 - 0 - 0 32", "S 2 poll 0102 - 0 - 0 32", 1)
        expect("a first transaction on a data lane fails", lambda: three("lane", qio_first, "lane.json"), refuse=True)
        repeated = "P 2 02002a00 4 00000003 11\nS 2 poll 0100 - 0 - 0 32\nS 2 poll 0000 - 0 - 0 32\n"
        expect(
            "a repeated transfer number fails",
            lambda: three("dup", text.replace(repeated, repeated + repeated, 1), "dup.json"),
            refuse=True,
        )
        expect(
            "a capture with no done line fails",
            lambda: three("nodone", text.replace("KNOB DONE\n", ""), "nodone.json"),
            refuse=True,
        )

        def after_done():
            three("after", text + "KNOB INFO after\n", "after.json")
            if "KNOB INFO after" in (root / "after.json").read_text():
                bad("present")

        expect("the line after the done line is not a mark", after_done)

        alien = root / "alien"
        write_run(alien, text, refs)
        serial_file = alien / "traces" / "serial.csv"
        serial_file.write_text(serial_file.read_text().replace(",u1,USB_SERIAL_JTAG,", ",u1,UART0,", 1))
        expect("a serial byte that is not USB_SERIAL_JTAG fails", lambda: panel(alien, refs_path, False), refuse=True)

        bare_net = {
            "analysis": {"transient": {"stop_ps": 1, "max_step_ps": 1, "grid_ps": 1}},
            "parts": [
                {"name": "u1", "kind": "esp32s3", "console": [{"t_ps": 100_000_000_000, "bytes": [103]}]},
                {"name": "sw_a", "kind": "switch", "initial": "open", "events": []},
                {"name": "sw_b", "kind": "switch", "initial": "open", "events": []},
                {"name": "u2", "kind": "cst816", "touches": []},
            ],
        }
        zero = dict(refs)
        zero["events"] = ["T down 0"]
        zero_path = root / "zero.json"
        zero_path.write_text(json.dumps(zero))
        zero_net = root / "zero-net.json"
        zero_net.write_text(json.dumps(bare_net))
        expect("T down 0 fails", lambda: script(zero_path, zero_net), refuse=True)

        def close_touches():
            saved = esp32s3_knob.TOUCH_HOLD_PS
            esp32s3_knob.TOUCH_HOLD_PS = MIN_GAP_PS - 1
            gap_net = root / "gap-net.json"
            gap_net.write_text(json.dumps(bare_net))
            try:
                script(refs_path, gap_net)
            finally:
                esp32s3_knob.TOUCH_HOLD_PS = saved

        expect("touches under 2000 ps apart fail", close_touches, refuse=True)

        def before_start():
            raw = b"\xff\xfe" + text.encode("ascii")
            paths = []
            for index in range(3):
                path = root / f"bin-{index}.txt"
                path.write_bytes(raw)
                paths.append(path)
            write_refs(paths, root / "bin.json", DEFAULT_START)

        expect("bytes before the start mark are ignored", before_start)

        def non_ascii():
            raw = text.encode("ascii").replace(b"KNOB INFO sample", b"KNOB INFO sampl\xff", 1)
            paths = []
            for index in range(3):
                path = root / f"asc-{index}.txt"
                path.write_bytes(raw)
                paths.append(path)
            write_refs(paths, root / "asc.json", DEFAULT_START)

        expect("a section that is not ASCII fails", non_ascii, refuse=True)
        plain_addr = text.replace("KNOB INFO sample\n", "KNOB INFO sample\nKNOB INFO token 001122334455\n", 1)
        expect("a 12-hex address token fails", lambda: three("addr", plain_addr, "addr.json"), refuse=True)
        colon_addr = text.replace("KNOB INFO sample\n", "KNOB INFO sample\naa:bb:cc:dd:ee:ff\n", 1)
        expect("a colon-form address fails", lambda: three("colon", colon_addr, "colon.json"), refuse=True)

        seen_b = 0
        kept_lines = []
        for line in text.splitlines():
            if line.startswith("B "):
                seen_b += 1
                if seen_b == 2:
                    continue
            kept_lines.append(line)
        expect(
            "a short colour write fails",
            lambda: three("shortc", "\n".join(kept_lines) + "\n", "shortc.json"),
            refuse=True,
        )

        no_bytes = "\n".join(line for line in text.splitlines() if not line.startswith("B ")) + "\n"

        def colour_after():
            try:
                three("dump", no_bytes, "dump.json")
            except ValueError as error:
                if "a colour write after the dump ended" not in str(error):
                    bad(error)
                return
            bad("accepted")

        expect("a colour write after the dump ended is refused", colour_after)

        one = _write(root / "one.txt", text)
        two = _write(root / "two.txt", text)
        three_path = _write(root / "three.txt", text)
        expect("one capture fails", lambda: write_refs([one], root / "one.json", DEFAULT_START), refuse=True)
        expect("two captures fail", lambda: write_refs([one, two], root / "two.json", DEFAULT_START), refuse=True)
        expect(
            "four captures fail",
            lambda: write_refs([one, two, three_path, one], root / "four.json", DEFAULT_START),
            refuse=True,
        )
        expect(
            "an existing refs path fails",
            lambda: write_refs([one, two, three_path], refs_path, DEFAULT_START),
            refuse=True,
        )
        expect(
            "a refs path that is a capture fails",
            lambda: write_refs([one, two, three_path], one, DEFAULT_START),
            refuse=True,
        )
        expect(
            "the same capture path twice fails",
            lambda: write_refs([one, one, two], root / "same.json", DEFAULT_START),
            refuse=True,
        )
        link = root / "link.txt"
        link.symlink_to(one)
        expect(
            "two paths that resolve to one file fail",
            lambda: write_refs([one, link, two], root / "link.json", DEFAULT_START),
            refuse=True,
        )

        def colour_token(suffix: str, dest: str):
            first, rest = text.split("\nB 4 ", 1)
            payload, after = rest.split("\n", 1)
            return three(dest, first + "\nB 4 " + payload + suffix + "\n" + after, dest + ".json")

        expect("a 12-hex token on a colour line is kept", lambda: colour_token(" 001122334455", "btoken"))
        expect("a colon-form token on a colour line is kept", lambda: colour_token(" aa:bb:cc:dd:ee:ff", "bcolon"))

        def line_token(old: str, new: str, dest: str):
            try:
                three(dest, text.replace(old, new, 1), dest + ".json")
            except ValueError as error:
                if "12-hex address token" not in str(error):
                    bad(error)
                return
            bad("accepted")

        expect(
            "a 12-hex token on a K line fails", lambda: line_token("K a=1 b=0\n", "K a=1 b=0 001122334455\n", "kaddr")
        )
        expect("a 12-hex token on a T line fails", lambda: line_token("T down 1\n", "T down 1 001122334455\n", "taddr"))

        def min_flag():
            sink = io.StringIO()
            old = sys.stderr
            sys.stderr = sink
            try:
                code = main(
                    [
                        "refs",
                        "--write",
                        "--min",
                        "1",
                        str(one),
                        str(two),
                        str(three_path),
                        str(root / "min.json"),
                    ]
                )
            except SystemExit as error:
                code = error.code if isinstance(error.code, int) else 2
            finally:
                sys.stderr = old
            if code == 0 or "unrecognized arguments: --min" not in sink.getvalue():
                bad(sink.getvalue() or code)

        expect("a --min flag fails", min_flag)
