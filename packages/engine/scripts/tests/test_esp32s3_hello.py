"""What `esp32s3_hello` accepts and what it refuses."""

import json

from esp32s3_hello import CONSOLE, COUNTDOWN, SECOND_PS, TICK_PS, check_manifest, check_text, check_times, lines, masked


def expect(name: str, ok: bool) -> None:
    assert ok, name


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def serial_of(timed: list[tuple[int, str]]) -> str:
    rows = "".join(f"{at},u1,USB_SERIAL_JTAG,{ord(c)}\n" for at, line in timed for c in line)
    return "t_ps,part,usart,byte\n" + rows


def test_console_text_and_times() -> None:

    want = (
        "I (T) cpu_start: Unicore app\r\nRestarting in 1 seconds...\r\n"
        "Restarting in 0 seconds...\r\nRestarting now.\r\n"
    )
    timed = [
        (5, "I (57) cpu_start: Unicore app\r\n"),
        (7 * TICK_PS, "Restarting in 1 seconds...\r\n"),
        (7 * TICK_PS + SECOND_PS, "Restarting in 0 seconds...\r\n"),
        (7 * TICK_PS + 2 * SECOND_PS, "Restarting now.\r\n"),
    ]
    restart = [{"code": "chip.restart", "severity": "warning", "t_ps": 7 * TICK_PS + 2 * SECOND_PS}]
    got = lines(serial_of(timed))
    expect("a good run passes the text", not raises(lambda: check_text(got, want)))
    expect("a good run passes the times", not raises(lambda: check_times(got, restart)))
    expect("log times are masked", masked("W (12) x\r\nI (3) y\r\n") == "W (T) x\r\nI (T) y\r\n")
    expect("a changed byte fails", raises(lambda: check_text(got, want.replace("Unicore", "Unicorn"))))
    expect("a lost carriage return fails", raises(lambda: check_text(got, want.replace("\r", ""))))
    late = list(timed)
    late[2] = (late[2][0] + 2 * TICK_PS, late[2][1])
    expect("a late countdown line fails", raises(lambda: check_times(lines(serial_of(late)), restart)))
    first = list(timed)
    first[1] = (first[1][0] + 2 * TICK_PS, first[1][1])
    expect("a late first countdown line fails", raises(lambda: check_times(lines(serial_of(first)), restart)))
    expect("an empty console fails", raises(lambda: check_text([], want)))
    expect("a truncated console fails", raises(lambda: check_text(got[:-1], want)))
    expect("a reordered console fails", raises(lambda: check_text([got[0], got[2], got[1], got[3]], want)))
    early = [{**restart[0], "t_ps": restart[0]["t_ps"] - 2 * TICK_PS}]
    expect("an early restart fails", raises(lambda: check_times(got, early)))
    expect("a run without the restart fails", raises(lambda: check_times(got, [])))
    error = [{"code": "chip.clock", "severity": "error", "t_ps": 0}, *restart]
    expect("an error fails", raises(lambda: check_times(got, error)))
    other = serial_of(timed).replace("USB_SERIAL_JTAG", "USART0", 1)
    expect("another port fails", raises(lambda: lines(other)))
    netlist = {
        "parts": [
            {
                "kind": "esp32s3",
                "rom": {"sha256": "r", "bytes": 1},
                "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
            }
        ]
    }
    listed = {
        "blobs": [{"role": "rom", "sha256": "r", "bytes": 1}, {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8}]
    }
    expect("the manifest passes", not raises(lambda: check_manifest(listed, netlist)))
    unplaced = {"blobs": [listed["blobs"][0], {**listed["blobs"][1], "offset": None}]}
    expect("a region without its offset fails", raises(lambda: check_manifest(unplaced, netlist)))
    expect(
        "the committed console holds the countdown",
        len(COUNTDOWN.findall(json.loads(CONSOLE.read_text())["transcript"])) == 11,
    )
