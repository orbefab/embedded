"""What `esp32s3_blink` accepts and what it refuses."""

from esp32s3_blink import LATENCY, PS_PER_CYCLE, SOURCE, check_edges, check_serial, closed_form, expected_text


def vcd_of(changes: list[tuple[int, str]]) -> str:
    head = "$timescale 1 ps $end\n$scope module u1 $end\n$var wire 1 ! GPIO21 $end\n"
    head += "$upscope $end\n$enddefinitions $end\n#0\n$dumpvars\n0!\n$end\n"
    return head + "".join(f"#{at}\n{level}!\n" for at, level in changes)


def serial_of(text: str, stamp) -> str:
    out = "t_ps,part,usart,byte\n"
    for n, line in enumerate(text.splitlines(keepends=True)):
        for byte in line.encode():
            out += f"{stamp(n)},u1,USB_SERIAL_JTAG,{byte}\n"
    return out


def must_fail(what: str, run) -> None:
    try:
        run()
    except ValueError:
        return
    raise AssertionError(f"{what} passed")


def test_edges_and_serial() -> None:
    text = expected_text(SOURCE.read_text())
    period, edges = closed_form(text)
    good = [((k * period + LATENCY) * PS_PER_CYCLE, "1" if k % 2 else "0") for k in range(1, edges + 1)]
    check_edges(vcd_of(good), period, edges)
    check_serial(serial_of(text, lambda n: (n + 1) * PS_PER_CYCLE), text)

    moved = good[:3] + [(good[3][0] + PS_PER_CYCLE, good[3][1])] + good[4:]
    must_fail("an edge a cycle late", lambda: check_edges(vcd_of(moved), period, edges))
    must_fail("a missing edge", lambda: check_edges(vcd_of(good[:-1]), period, edges))
    late = [(at + PS_PER_CYCLE, level) for at, level in good]
    must_fail("every edge a cycle late", lambda: check_edges(vcd_of(late), period, edges))
    split = serial_of(text, lambda n: (n + 1) * PS_PER_CYCLE).replace(
        f"{PS_PER_CYCLE},u1,USB_SERIAL_JTAG,69", f"{2 * PS_PER_CYCLE},u1,USB_SERIAL_JTAG,69", 1
    )
    must_fail("a line over two packets", lambda: check_serial(split, text))
    must_fail("a changed byte", lambda: check_serial(serial_of(text.replace("END", "End"), lambda n: 0), text))
    must_fail("a wrong systimer line", lambda: closed_form(text.replace("00aae600", "00aae601")))
    must_fail("a wrong level", lambda: closed_form(text.replace("00000002 in 00000001", "00000002 in 00000000")))
