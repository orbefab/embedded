"""What `esp32s3_pulls` accepts and what it refuses."""

import json
from pathlib import Path

import pytest
from esp32s3_pulls import BOARD_TOLERANCE, Pad, Probe, check, check_board, check_ratio, check_static, lines

R = 45000.0


def value(v: float, unit: str) -> dict:
    return {"value": v, "unit": unit, "source": "user"}


def part(up: float = R, down: float | None = R) -> dict:
    out = {
        "name": "u1",
        "kind": "esp32s3",
        "pins": {"VCC": "vcc", "GND": "gnd", "GPIO44": "pad"},
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
        "output": {
            "high_drop": value(0.0, "V"),
            "high_resistance": value(16.5, "ohm"),
            "low_drop": value(0.0, "V"),
            "low_resistance": value(17.68, "ohm"),
        },
        "pull_up_resistance": value(up, "ohm"),
        "leakage_resistance": value(66e6, "ohm"),
        "input_threshold": {"low": value(0.25, "1"), "high": value(0.75, "1")},
    }
    if down is not None:
        out["pull_down_resistance"] = value(down, "ohm")
    return out


def console(
    static: tuple[int, int, int, int] = (1, 0, 0, 0),
    fall: int = 1030,
    rise: int = 1030,
    base: int = 30,
    poll: int = 5000,
    reps: int = 16,
    done: bool = True,
) -> list[str]:
    out = ["I (10) boot: noise", "pulls ready"]
    out += [f"static {name} {level}" for name, level in zip(("up", "down", "both", "none"), static, strict=True)]
    out += [f"poll 1000 {poll}"]
    out += [f"rep {k} base {base} fall {fall} rise {rise}" for k in range(reps)]
    if done:
        out.append("PULLS DONE")
    return out


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def test_equal_pulls_give_a_ratio_of_one_and_the_last_level_between_the_thresholds() -> None:
    pad = Pad({"parts": [part()]})
    assert pad.levels() == {"up": 1, "down": 0, "both": 0, "none": 0}
    assert pad.ratio() == pytest.approx(1.0, abs=1e-3)


def test_a_stronger_pull_up_reads_high_with_both() -> None:
    pad = Pad({"parts": [part(up=10000.0)]})
    assert pad.levels() == {"up": 1, "down": 0, "both": 1, "none": 1}
    # The leakage moves it 0.11 % off the pulls' ratio.
    assert pad.ratio() == pytest.approx(4.5, rel=2e-3)


def test_static_levels() -> None:
    pad = Pad({"parts": [part()]})
    assert not raises(lambda: check_static(Probe(console()), pad))
    assert raises(lambda: check_static(Probe(console(static=(1, 0, 1, 0))), pad))
    assert raises(lambda: check_static(Probe(console(static=(0, 0, 0, 0))), pad))


def test_the_ratio_within_a_poll_and_the_largest_base() -> None:
    pad = Pad({"parts": [part()]})
    # One poll is 5 cycles and the base 30, so each edge of about 1000 cycles is good to 3.5 %.
    assert not raises(lambda: check_ratio(Probe(console()), pad))
    assert not raises(lambda: check_ratio(Probe(console(fall=1090)), pad))
    assert raises(lambda: check_ratio(Probe(console(fall=1110)), pad))
    assert raises(lambda: check_ratio(Probe(console(rise=1110)), pad))


def test_a_pull_down_twice_the_pull_up_wants_twice_the_fall() -> None:
    pad = Pad({"parts": [part(down=2 * R)]})
    assert pad.ratio() == pytest.approx(2.0, rel=1e-3)
    assert not raises(lambda: check_ratio(Probe(console(fall=2030)), pad))
    assert raises(lambda: check_ratio(Probe(console()), pad))


def test_edges_of_a_few_polls_widen_the_bound() -> None:
    _, bound = Probe(console(fall=40, rise=40)).ratio()
    assert bound > 1


def test_the_board() -> None:
    run = Probe(console())
    assert not raises(lambda: check_board(run, Probe(console())))
    # The run's bound and the board's are 7 % each, so 24 % with the tolerance.
    near = 30 + round(1000 * (1 + BOARD_TOLERANCE + 0.10))
    assert not raises(lambda: check_board(run, Probe(console(fall=near))))
    far = 30 + round(1000 * 1.5)
    assert raises(lambda: check_board(run, Probe(console(fall=far))))


def test_a_broken_console_fails() -> None:
    assert raises(lambda: Probe(console(done=False)))
    assert raises(lambda: Probe(console(reps=15)))
    assert raises(lambda: Probe(console(fall=0)))
    assert raises(lambda: Probe(console(base=2000)).ratio())
    assert raises(lambda: Probe(console()[:2] + console()[3:]))
    swapped = console()
    swapped[8], swapped[9] = swapped[9], swapped[8]
    assert raises(lambda: Probe(swapped))


def test_a_part_without_a_pull_down_fails() -> None:
    assert raises(lambda: Pad({"parts": [part(down=None)]}))


def write_run(path: Path, text: list[str], errors: bool = False) -> None:
    netlist = {"parts": [part()]}
    (path / "traces").mkdir(parents=True)
    (path / "netlist.json").write_text(json.dumps(netlist))
    rows = "".join(f"{k},u1,USB_SERIAL_JTAG,{ord(c)}\n" for k, c in enumerate("\n".join(text) + "\n"))
    (path / "traces" / "serial.csv").write_text("t_ps,part,usart,byte\n" + rows)
    diagnostics = [{"code": "chip.halted", "severity": "error"}] if errors else []
    (path / "diagnostics.json").write_text(json.dumps(diagnostics))
    blobs = [{"role": "rom", "sha256": "r", "bytes": 1}, {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8}]
    (path / "manifest.json").write_text(json.dumps({"blobs": blobs}))


def test_a_run_folder(tmp_path: Path) -> None:
    example = tmp_path / "example"
    example.mkdir()
    write_run(tmp_path / "good", console())
    check(tmp_path / "good", example)
    (example / "board.txt").write_text("\n".join(console()) + "\n")
    check(tmp_path / "good", example)
    (example / "board.txt").write_text("\n".join(console(fall=3000)) + "\n")
    assert raises(lambda: check(tmp_path / "good", example))
    write_run(tmp_path / "error", console(), errors=True)
    assert raises(lambda: check(tmp_path / "error", tmp_path / "none"))


def test_another_port_fails() -> None:
    text = "t_ps,part,usart,byte\n0,u1,USART0,65\n"
    assert raises(lambda: lines(text))
