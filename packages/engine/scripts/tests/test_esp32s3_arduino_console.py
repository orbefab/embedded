"""What `esp32s3_arduino_console` accepts and what it refuses."""

from esp32s3_arduino_console import (
    LAST_TICK,
    check_diagnostics,
    check_echo,
    check_manifest,
    check_ticks,
    echoed_phrase,
    lines,
)


def expect(name: str, ok: bool) -> None:
    assert ok, name


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def serial_of(text: str) -> str:
    rows = "".join(f"0,u1,USB_SERIAL_JTAG,{ord(c)}\n" for c in text)
    return "t_ps,part,usart,byte\n" + rows


def transcript(end: str = "\n", ticks_through: int = LAST_TICK, core: int = 1, extra: str = "") -> str:
    body = "".join(f"tick {k} core {core}{end}" for k in range(ticks_through + 1))
    return body + extra


def test_ticks_and_echoes() -> None:

    good = lines(serial_of("boot\n" + transcript()))
    expect("ticks 0 to 10 pass", not raises(lambda: check_ticks(good)))
    crlf = lines(serial_of(transcript(end="\r\n")))
    expect("a carriage return passes", not raises(lambda: check_ticks(crlf)))
    short = lines(serial_of(transcript(ticks_through=9)))
    expect("a missing tick 10 fails", raises(lambda: check_ticks(short)))
    other = lines(serial_of(transcript(core=0)))
    expect("core 0 fails", raises(lambda: check_ticks(other)))
    netlist = {"parts": [{"kind": "esp32s3", "console": [{"t_ps": 1, "bytes": [65, 66]}]}]}
    echoed_text = lines(serial_of(transcript(extra="got 65\ngot 66\n")))
    expect("the echoed bytes pass", not raises(lambda: check_echo(echoed_text, netlist)))
    missed = lines(serial_of(transcript(extra="got 65\n")))
    expect("a missing echo fails", raises(lambda: check_echo(missed, netlist)))
    expect(
        "an error fails",
        raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])),
    )
    expect(
        "a warning passes",
        not raises(lambda: check_diagnostics([{"code": "chip.unmodeled_register", "severity": "warning"}])),
    )
    foreign = serial_of(transcript()).replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(foreign)))
    part = {
        "kind": "esp32s3",
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
    }
    listed = {
        "blobs": [
            {"role": "rom", "sha256": "r", "bytes": 1},
            {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8},
        ]
    }
    expect("the manifest passes", not raises(lambda: check_manifest(listed, {"parts": [part]})))
    expect("one byte is singular", echoed_phrase(1) == "1 byte echoed")
    expect("two bytes stay plural", echoed_phrase(2) == "2 bytes echoed")
