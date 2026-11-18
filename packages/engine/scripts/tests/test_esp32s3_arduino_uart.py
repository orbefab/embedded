"""What `esp32s3_arduino_uart` accepts and what it refuses."""

from esp32s3_arduino_console import LAST_TICK, check_diagnostics, check_ticks
from esp32s3_arduino_uart import check_echo, frame_ps, lines, source_starts, strip_source


def expect(name: str, ok: bool) -> None:
    assert ok, name


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def netlist() -> dict:
    return {
        "parts": [
            {
                "kind": "voltage_source",
                "voltage": {
                    "kind": "serial",
                    "events": [{"t_ps": 300_000_000_000, "baud": 115200, "frame": "8N1", "bytes": [103]}],
                },
            }
        ]
    }


def uart_rows(text: str, t_ps: int = 0) -> list[dict]:
    return [{"t_ps": str(t_ps), "part": "u1", "usart": "UART0", "byte": str(ord(c))} for c in text]


def serial_of(text: str, extra: list[dict] | None = None) -> str:
    rows = uart_rows(text, t_ps=1_000_000_000)
    if extra:
        rows = extra + rows
    body = "".join(f"{row['t_ps']},u1,{row['usart']},{row['byte']}\n" for row in rows)
    return "t_ps,part,usart,byte\n" + body


def transcript(end: str = "\n", ticks_through: int = LAST_TICK, core: int = 1, extra: str = "") -> str:
    body = "".join(f"tick {k} core {core}{end}" for k in range(ticks_through + 1))
    return body + extra


def test_source_byte_is_dropped_once() -> None:
    board = netlist()
    start, baud, byte = source_starts(board)[0]
    expect("the frame is ten bit times", frame_ps(baud) == (10 * 10**12) // baud)
    expect("the byte starts at the event", start == 300_000_000_000 and byte == 103)
    rows = [
        {"t_ps": str(start + 1000), "part": "u1", "usart": "UART0", "byte": "103"},
        {"t_ps": str(start + frame_ps(baud) + 1), "part": "u1", "usart": "UART0", "byte": "103"},
    ]
    kept = strip_source(rows, source_starts(board))
    expect("the later copy stays", len(kept) == 1 and int(kept[0]["t_ps"]) == start + frame_ps(baud) + 1)
    early = [{"t_ps": str(start - frame_ps(baud) - 1), "part": "u1", "usart": "UART0", "byte": "103"}]
    expect("a byte before the frame stays", strip_source(early, source_starts(board)) == early)


def test_ticks_and_echoes() -> None:
    board = netlist()
    good = lines(serial_of("boot\n" + transcript(extra="got 103\n")), board)
    expect("ticks 0 to 10 pass", not raises(lambda: check_ticks(good)))
    expect("the echoed byte passes", not raises(lambda: check_echo(good, board)))
    short = lines(serial_of(transcript(ticks_through=9)), board)
    expect("a missing tick 10 fails", raises(lambda: check_ticks(short)))
    missed = lines(serial_of(transcript()), board)
    expect("a missing echo fails", raises(lambda: check_echo(missed, board)))
    start = source_starts(board)[0][0]
    rx = [{"t_ps": str(start), "part": "u1", "usart": "UART0", "byte": "103"}]
    with_rx = lines(serial_of(transcript(extra="got 103\n"), extra=rx), board)
    expect("the source byte is not an extra line", not raises(lambda: check_echo(with_rx, board)))
    foreign = serial_of(transcript()).replace("UART0", "USART0", 1)
    expect("another port fails", raises(lambda: lines(foreign, board)))
    expect(
        "an error fails",
        raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])),
    )
