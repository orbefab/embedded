"""What `esp32s3_arduino_adc` accepts and what it refuses."""

from esp32s3_adc import SETS
from esp32s3_arduino_adc import check_readings, readings
from test_esp32s3_adc import MODEL, netlist


def console(raw: int, mv: int, passes: int = 8, done: bool = True) -> list[str]:
    out = ["E (10) noise"] + [f"adc {k} raw {raw} mv {mv}" for k in range(passes)]
    if done:
        out.append("ADC DONE")
    return out


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def test_the_11_db_line_passes_and_another_attenuations_fails() -> None:
    raw = MODEL[3].code(1650)
    good = console(raw, MODEL[3].millivolts(raw))
    assert not raises(lambda: check_readings(readings(good), netlist(), SETS.read_text()))
    # The raw code 6 dB reads, with its own calibration's millivolts.
    six = MODEL[2].code(1650)
    assert raises(lambda: check_readings(readings(console(six, MODEL[2].millivolts(six))), netlist(), SETS.read_text()))
    assert raises(lambda: check_readings(readings(console(raw, 1650 + 7)), netlist(), SETS.read_text()))


def test_a_console_without_its_lines_fails() -> None:
    assert raises(lambda: readings(console(1779, 1650, done=False)))
    assert raises(lambda: readings(console(1779, 1650, passes=7)))
