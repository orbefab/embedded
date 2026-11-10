"""What `esp32s3_echo` accepts and what it refuses."""

from esp32s3_echo import US, check_diagnostics, check_manifest, check_timing, events, got_packets, lines, ready_us


def expect(name: str, ok: bool) -> None:
    assert ok, name


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def serial_of(text: str, at: int = 0) -> str:
    rows = "".join(f"{at},u1,USB_SERIAL_JTAG,{ord(c)}\n" for c in text)
    return "t_ps,part,usart,byte\n" + rows


def test_echo_window() -> None:

    t_us = 1_000_000
    n, rest, byte = 2, 500, 65
    a_us = t_us + n * 1000 + rest
    a_ps = a_us * US
    sent = [(a_ps, byte)]
    earliest = (t_us - 100 + (n + 1) * 1000) * US

    def packet(got_ps: int) -> list[tuple[int, int]]:
        return [(got_ps, byte)]

    expect("an echo at the byte passes", not raises(lambda: check_timing(t_us, sent, packet(a_ps))))
    expect(
        "an echo 100 µs later passes",
        not raises(lambda: check_timing(t_us, sent, packet(a_ps + 100 * US))),
    )
    early = packet(a_ps - 90 * US)
    expect("an echo 90 µs before its byte fails", raises(lambda: check_timing(t_us, sent, early)))
    late = packet(earliest + US)
    expect("an echo 1 µs after the next tick fails", raises(lambda: check_timing(t_us, sent, late)))
    misplaced = [(t_us + n * 1000 + 900) * US, byte]
    expect(
        "a byte 900 µs past the millisecond fails",
        raises(lambda: check_timing(t_us, [misplaced], packet(misplaced[0]))),
    )
    expect("the wrong byte fails", raises(lambda: check_timing(t_us, sent, [(a_ps, byte + 1)])))
    text = serial_of(f"ready {t_us}\n") + serial_of(f"got {byte}\n", a_ps).split("\n", 1)[1]
    parsed = lines(text)
    expect("ready is the printed t", ready_us(parsed) == t_us)
    expect("got keeps its byte", got_packets(parsed) == [(a_ps, byte)])
    expect("a missing ready line fails", raises(lambda: ready_us(lines(serial_of("boot\n")))))
    expect(
        "an error fails",
        raises(lambda: check_diagnostics([{"code": "chip.clock", "severity": "error"}])),
    )
    expect(
        "a warning passes",
        not raises(lambda: check_diagnostics([{"code": "chip.unmodeled_register", "severity": "warning"}])),
    )
    other = serial_of(f"ready {t_us}\n").replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(other)))
    part = {
        "kind": "esp32s3",
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
        "console": [{"t_ps": a_ps, "bytes": [byte]}],
    }
    listed = {
        "blobs": [
            {"role": "rom", "sha256": "r", "bytes": 1},
            {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8},
        ]
    }
    expect("the manifest passes", not raises(lambda: check_manifest(listed, {"parts": [part]})))
    expect("the console event is the byte", events({"parts": [part]}) == sent)
