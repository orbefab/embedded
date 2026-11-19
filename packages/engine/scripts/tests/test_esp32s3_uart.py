"""What `esp32s3_uart` accepts and what it refuses."""

from pathlib import Path

from esp32s3_uart import (
    bit_lengths,
    board_tolerance,
    burst_span,
    check_board,
    check_console,
    check_uart_rows,
    edges_in_frame,
    expected_edges,
    frame_edges,
    program,
    script_rows,
    split_serial,
    transfers,
)

SOURCE = Path(__file__).resolve().parents[2] / "tests/firmware/esp32s3/idf/uart/main/uart_main.c"


def expect(name: str, ok: bool) -> None:
    assert ok, name


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def test_burst_fraction_passes_the_poll_bound() -> None:
    spec = program(SOURCE.read_text())
    expect("the source names one burst", len(spec["bursts"]) == 1)
    burst = spec["bursts"][0]
    _baud, div, frag = next(item for item in spec["bauds"] if item[0] == burst["baud"])
    expect("the burst baud carries a fraction", frag != 0)
    with_frag = frame_edges(burst["cfg"], burst["byte"], burst["count"], div, frag, spec["apb_hz"], spec["cpu_hz"])
    without = frame_edges(burst["cfg"], burst["byte"], burst["count"], div, 0, spec["apb_hz"], spec["cpu_hz"])
    expect("the burst has an edge for every change", len(with_frag) == len(without) and len(with_frag) > 1)
    late = [abs(got - want) for (got, _), (want, _) in zip(with_frag, without, strict=True)]
    expect("a later burst edge is past the poll bound without the fraction", max(late) > spec["bound"])
    sent = transfers(spec)
    lines = _console(spec)
    # The helper prints one byte at a time. The burst lines follow the 115200 cases.
    burst_at = next(i for i, line in enumerate(lines) if line == "baud 1000000")
    burst_lines = ["burst"]
    for at, level in with_frag:
        burst_lines.append(f"edge {at} {level}")
    for _ in range(burst["count"]):
        burst_lines.append(f"tx {burst['byte']:02x}")
        burst_lines.append(f"rx {burst['byte']:02x}")
    lines[burst_at:burst_at] = burst_lines
    expect("the burst console passes", not raises(lambda: check_console(lines, sent, spec["bound"])))
    bad = []
    seen = False
    replaced = 0
    for line in lines:
        if line == "burst":
            seen = True
        if seen and line.startswith("edge ") and replaced < len(without):
            at, level = without[replaced]
            bad.append(f"edge {at} {level}")
            replaced += 1
            continue
        bad.append(line)
    expect("every burst edge was rewritten", replaced == len(without))
    expect("dropping the fraction fails the burst", raises(lambda: check_console(bad, sent, spec["bound"])))


def test_divider_and_edges() -> None:
    spec = program(SOURCE.read_text())
    lengths = bit_lengths(694, 7, 16, spec["apb_hz"], spec["cpu_hz"])
    expect("nine bits stay 694 APB clocks", lengths.count(694 * 3) == 9)
    expect("seven bits grow by one APB clock", lengths.count(695 * 3) == 7)
    edges = expected_edges("8N1", 0x55, 80, 0, spec["apb_hz"], spec["cpu_hz"])
    expect("0x55 at 1 Mbaud changes every bit", len(edges) == 10)
    expect("the start bit is cycle 0", edges[0] == (0, 0))
    expect("each 1 Mbaud bit is 240 CPU cycles", edges[1][0] == 240)


def _console(spec: dict, slip: int = 0) -> list[str]:
    lines = ["I (59) cpu_start: Unicore app"]
    for baud, div, frag in spec["bauds"]:
        lines.append(f"baud {baud}")
        for cfg, values in spec["cases"]:
            lines.append(f"cfg {cfg}")
            for byte in values:
                lines.append(f"tx {byte:02x}")
                lines.append(f"rx {byte:02x}")
                for at, level in expected_edges(cfg, byte, div, frag, spec["apb_hz"], spec["cpu_hz"]):
                    lines.append(f"edge {at + slip} {level}")
    lines.append("UART DONE")
    return lines


def test_console_window() -> None:
    spec = program(SOURCE.read_text())
    sent = []
    for baud, div, frag in spec["bauds"]:
        for cfg, values in spec["cases"]:
            for byte in values:
                sent.append(
                    {
                        "baud": baud,
                        "cfg": cfg,
                        "byte": byte,
                        "edges": expected_edges(cfg, byte, div, frag, spec["apb_hz"], spec["cpu_hz"]),
                    }
                )
    lines = _console(spec)
    expect("the closed form passes", not raises(lambda: check_console(lines, sent, spec["bound"])))
    late = _console(spec, slip=spec["bound"])
    expect("an offset on the bound fails", raises(lambda: check_console(late, sent, spec["bound"])))
    wrong = [line if line != "rx 00" else "rx 01" for line in lines]
    expect("the wrong byte fails", raises(lambda: check_console(wrong, sent, spec["bound"])))


def _offsets(spec: dict, div: int, frag: int, bits: int) -> list[int]:
    total = 0
    out = []
    for length in bit_lengths(div, frag, bits, spec["apb_hz"], spec["cpu_hz"]):
        total += length
        out.append(total)
    return out


def _slide(lines: list[str], baud: int, tx: int, delta: int, *, edge: int | None = None) -> list[str]:
    """Add `delta` to one frame's edges after `tx` at `baud`. `edge` selects one of them."""
    out = []
    current = None
    sent = None
    armed = False
    index = 0
    hits = 0
    for line in lines:
        if line.startswith("baud "):
            current = int(line.split()[1])
            armed = False
        elif line == "burst":
            armed = False
        elif line.startswith("tx "):
            sent = int(line.split()[1], 16)
            armed = False
        elif line.startswith("edge ") and current == baud and sent == tx:
            if not armed:
                index = 0
                armed = True
            at, level = line.split()[1:]
            if edge is None or index == edge:
                at = str(int(at) + delta)
                hits += 1
            index += 1
            out.append(f"edge {at} {level}")
            continue
        else:
            armed = False
        out.append(line)
    if hits == 0:
        raise AssertionError(f"no edge at baud {baud} tx {tx:02x}")
    return out


def test_board_intervals() -> None:
    spec = program(SOURCE.read_text())
    lines = _console(spec)
    body = "\n".join(lines) + "\n"
    tolerance = board_tolerance(spec["bound"])

    def same() -> None:
        check_board(lines, body, spec, tolerance)

    expect("a good board passes", not raises(same))
    slid = _slide(lines, 115200, 0x55, 5000)
    expect(
        "a shifted frame start passes",
        not raises(lambda: check_board(lines, "\n".join(slid) + "\n", spec, tolerance)),
    )
    near = _slide(lines, 1000000, 0x55, tolerance - 1, edge=1)
    expect(
        "an interval one cycle inside the allowance passes",
        not raises(lambda: check_board(lines, "\n".join(near) + "\n", spec, tolerance)),
    )
    far = _slide(lines, 1000000, 0x55, tolerance, edge=1)
    expect(
        "an interval on the allowance fails",
        raises(lambda: check_board(lines, "\n".join(far) + "\n", spec, tolerance)),
    )
    expect(
        "a board byte that differs fails",
        raises(lambda: check_board(lines, body.replace("rx 00", "rx 01", 1), spec, tolerance)),
    )
    for baud, div, frag in spec["bauds"]:
        bit = bit_lengths(div, frag, 1, spec["apb_hz"], spec["cpu_hz"])[0]
        moved = _slide(lines, baud, 0x55, bit, edge=1)
        expect(
            f"a one-bit error at {baud} fails",
            raises(lambda moved=moved: check_board(lines, "\n".join(moved) + "\n", spec, tolerance)),
        )


def _with_burst(spec: dict, *, div_delta: int = 0, frag_delta: int = 0) -> list[str]:
    """The closed-form console, with the burst where the app prints it.

    `div_delta` and `frag_delta` retime every edge at the burst's baud. The other baud stays.
    """
    lines = ["I (59) cpu_start: Unicore app"]
    for baud, div, frag in spec["bauds"]:
        use_div, use_frag = div, frag
        if any(item["baud"] == baud for item in spec["bursts"]):
            use_div += div_delta
            use_frag += frag_delta
        lines.append(f"baud {baud}")
        for cfg, values in spec["cases"]:
            lines.append(f"cfg {cfg}")
            for byte in values:
                lines.append(f"tx {byte:02x}")
                lines.append(f"rx {byte:02x}")
                for at, level in expected_edges(cfg, byte, use_div, use_frag, spec["apb_hz"], spec["cpu_hz"]):
                    lines.append(f"edge {at} {level}")
        for burst in spec["bursts"]:
            if burst["baud"] != baud:
                continue
            edges = frame_edges(
                burst["cfg"], burst["byte"], burst["count"], use_div, use_frag, spec["apb_hz"], spec["cpu_hz"]
            )
            lines.append("burst")
            lines += [f"edge {at} {level}" for at, level in edges]
            for _ in range(burst["count"]):
                lines.append(f"tx {burst['byte']:02x}")
                lines.append(f"rx {burst['byte']:02x}")
    lines.append("UART DONE")
    return lines


def _burst_edges(lines: list[str]) -> list[tuple[int, int]]:
    start = lines.index("burst") + 1
    edges = []
    for line in lines[start:]:
        if not line.startswith("edge "):
            break
        at, level = line.split()[1:]
        edges.append((int(at), int(level)))
    return edges


def _refusal(fn) -> str:
    try:
        fn()
    except ValueError as error:
        return str(error)
    return ""


def _nudge(lines: list[str], after: str, index: int, delta: int) -> list[str]:
    """Add `delta` to edge `index` of the group that follows the line `after`."""
    out = []
    arm = False
    at = 0
    hit = False
    for line in lines:
        if line == after:
            arm = True
            at = 0
        elif arm and line.startswith("edge "):
            if at == index:
                offset, level = line.split()[1:]
                line = f"edge {int(offset) + delta} {level}"
                hit = True
            at += 1
        elif arm:
            arm = False
        out.append(line)
    if not hit:
        raise AssertionError(f"no edge {index} after {after!r}")
    return out


def test_poll_entry_is_not_a_uart_interval() -> None:
    spec = program(SOURCE.read_text())
    tolerance = board_tolerance(spec["bound"])
    lines = _with_burst(spec)

    def body(board: list[str]) -> None:
        check_board(lines, "\n".join(board) + "\n", spec, tolerance)

    expect("the burst console passes", not raises(lambda: body(lines)))
    cold = _slide(lines, 115200, 0x00, 2082, edge=1)
    expect("the cold frame's stop is not an interval", not raises(lambda: body(cold)))
    start = _nudge(lines, "burst", 0, 2082)
    expect("the burst's first start is the entry", not raises(lambda: body(start)))
    near = _nudge(lines, "burst", 1, tolerance - 1)
    expect("the first stop can sit one cycle inside the allowance", not raises(lambda: body(near)))
    entered = _nudge(lines, "burst", 1, 2082)
    expect("the first stop is in the burst span", "first stop" in _refusal(lambda: body(entered)))
    later = _nudge(lines, "burst", 3, tolerance)
    expect("the burst's next frame is the UART", raises(lambda: body(later)))


def test_divider_one_high_fails_the_console_and_the_burst_span() -> None:
    spec = program(SOURCE.read_text())
    sent = transfers(spec)
    tolerance = board_tolerance(spec["bound"])
    burst = spec["bursts"][0]
    good = _with_burst(spec)
    bad = _with_burst(spec, div_delta=1)
    grew = burst_span(_burst_edges(bad), burst["cfg"], burst["byte"]) - burst_span(
        _burst_edges(good), burst["cfg"], burst["byte"]
    )
    expect("a divider one high lengthens the burst span by 210", grew == 210)
    expect("the closed form rejects a divider one high", raises(lambda: check_console(bad, sent, spec["bound"])))
    refused = _refusal(lambda: check_board(good, "\n".join(bad) + "\n", spec, tolerance))
    expect("the burst span rejects a divider one high", "first stop" in refused)


def test_only_the_burst_span_catches_a_divider_one_high() -> None:
    spec = program(SOURCE.read_text())
    sent = transfers(spec)
    tolerance = board_tolerance(spec["bound"])
    burst = spec["bursts"][0]
    baud, div, frag = next(item for item in spec["bauds"] if item[0] == burst["baud"])
    base = frame_edges(burst["cfg"], burst["byte"], burst["count"], div, frag, spec["apb_hz"], spec["cpu_hz"])
    high = frame_edges(burst["cfg"], burst["byte"], burst["count"], div + 1, frag, spec["apb_hz"], spec["cpu_hz"])
    wide = max(got - want for (want, _), (got, _) in zip(base, high, strict=True)) + 1
    good = _with_burst(spec)
    bad = _with_burst(spec, div_delta=1)
    expect("the widened window is past the console bound", wide > spec["bound"])
    expect(
        "a widened console window accepts a divider one high",
        not raises(lambda: check_console(bad, sent, wide)),
    )
    refused = _refusal(lambda: check_board(good, "\n".join(bad) + "\n", spec, tolerance))
    expect("only the burst span rejects a divider one high", "first stop" in refused)


def test_fraction_one_high_passes_both_checks() -> None:
    spec = program(SOURCE.read_text())
    sent = transfers(spec)
    tolerance = board_tolerance(spec["bound"])
    burst = spec["bursts"][0]
    good = _with_burst(spec)
    bad = _with_burst(spec, frag_delta=1)
    base = _burst_edges(good)
    moved = _burst_edges(bad)
    expect(
        "a fraction one high moves the burst's last edge by 24",
        moved[-1][0] - base[-1][0] == 24,
    )
    expect(
        "a fraction one high lengthens the burst span by 21",
        burst_span(moved, burst["cfg"], burst["byte"]) - burst_span(base, burst["cfg"], burst["byte"]) == 21,
    )
    expect("a fraction one high passes the console", not raises(lambda: check_console(bad, sent, spec["bound"])))
    expect(
        "a fraction one high passes the board compare",
        not raises(lambda: check_board(good, "\n".join(bad) + "\n", spec, tolerance)),
    )


def test_a_bit_is_past_two_polls() -> None:
    spec = program(SOURCE.read_text())
    allowance = board_tolerance(spec["bound"])
    expect("the allowance is two polls", allowance == 2 * spec["bound"])
    slow = bit_lengths(694, 7, 1, spec["apb_hz"], spec["cpu_hz"])[0]
    fast = bit_lengths(80, 0, 1, spec["apb_hz"], spec["cpu_hz"])[0]
    expect("the 115200 divider's bit is 2082 cycles", slow == 2082)
    expect("the 1 Mbaud bit is 240 cycles", fast == 240)
    expect("both bits are past two polls", slow >= allowance and fast >= allowance)
    base = _offsets(spec, 694, 7, 9)
    divided = _offsets(spec, 695, 7, 9)
    fraction = _offsets(spec, 694, 8, 9)
    expect("a divider one high moves the ninth edge by 27", divided[-1] - base[-1] == 27)
    expect("a fraction one high moves a 115200 edge by 3", max(b - a for a, b in zip(base, fraction, strict=True)) == 3)
    one = bit_lengths(694, 7, 10, spec["apb_hz"], spec["cpu_hz"])
    high = bit_lengths(695, 7, 10, spec["apb_hz"], spec["cpu_hz"])
    frames = spec["bursts"][0]["count"]

    def last(lengths: list[int]) -> int:
        return (frames - 1) * sum(lengths) + sum(lengths[:9])

    expect("the burst's last edge moves 237 from the first start", last(high) - last(one) == 237)
    expect("one 0x00 frame is two edges", edges_in_frame("8N1", 0x00) == 2)
    expect("one 0x55 frame is ten edges", edges_in_frame("8N1", 0x55) == 10)


def test_uart_rows_ignore_the_boot_log() -> None:
    text = "boot\nbaud 115200\n"
    usb = []
    at = 1000
    for char in text:
        usb.append({"t_ps": str(at), "part": "u1", "usart": "USB_SERIAL_JTAG", "byte": str(ord(char))})
        if char == "\n":
            at = 9000
    uart = [
        {"t_ps": "1000", "part": "u1", "usart": "UART0", "byte": "73"},
        {"t_ps": "20000", "part": "u1", "usart": "UART0", "byte": "85"},
        {"t_ps": "20000", "part": "u1", "usart": "UART0", "byte": "85"},
    ]
    sent = [{"byte": 0x55}]
    rows = script_rows(uart, usb)
    expect("the boot byte is not one of the script rows", not raises(lambda: check_uart_rows(rows, sent)))
    foreign = "t_ps,part,usart,byte\n0,u1,USART0,1\n"
    expect("another port fails", raises(lambda: split_serial(foreign)))
