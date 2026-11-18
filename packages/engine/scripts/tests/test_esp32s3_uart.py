"""What `esp32s3_uart` accepts and what it refuses."""

from pathlib import Path

from esp32s3_uart import (
    BOARD_TOLERANCE,
    bit_lengths,
    check_board,
    check_console,
    check_uart_rows,
    expected_edges,
    program,
    script_rows,
    split_serial,
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
    body = "\n".join(lines) + "\n"
    near = body.replace("edge 0 0", f"edge {BOARD_TOLERANCE} 0", 1)
    expect("a board edge 32 cycles away passes", not raises(lambda: check_board(lines, near)))
    far = body.replace("edge 0 0", f"edge {BOARD_TOLERANCE + 1} 0", 1)
    expect("a board edge 33 cycles away fails", raises(lambda: check_board(lines, far)))
    expect("a board byte that differs fails", raises(lambda: check_board(lines, body.replace("rx 00", "rx 01", 1))))


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
