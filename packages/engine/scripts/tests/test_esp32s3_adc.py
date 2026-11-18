"""What `esp32s3_adc` accepts and what it refuses."""

import json
from pathlib import Path

import pytest
from esp32s3_adc import (
    SETS,
    Curve,
    Probe,
    check,
    check_board,
    check_probe,
    curves,
    digi,
    efuse_set,
    pad_volts,
)

SYNTHETIC = efuse_set("synthetic", SETS.read_text())


def value(v: float, unit: str) -> dict:
    return {"value": v, "unit": unit, "source": "user"}


def netlist(supply: float = 3.3, top: float = 10000.0, bottom: float = 10000.0, efuse: str | None = None) -> dict:
    chip = {
        "name": "u1",
        "kind": "esp32s3",
        "pins": {"VCC": "vcc", "GND": "gnd", "GPIO6": "adc"},
        "rom": {"sha256": "r", "bytes": 1},
        "firmware": {"flash": [{"offset": 8, "sha256": "f", "bytes": 2}]},
        "leakage_resistance": value(66e6, "ohm"),
    }
    if efuse is not None:
        chip["efuse"] = efuse
    return {
        "ground": "gnd",
        "parts": [
            {"name": "v1", "kind": "voltage_source", "pins": {"p": "vcc", "n": "gnd"}, "voltage": value(supply, "V")},
            chip,
            {"name": "r1", "kind": "resistor", "pins": {"a": "vcc", "b": "adc"}, "resistance": value(top, "ohm")},
            {"name": "r2", "kind": "resistor", "pins": {"a": "adc", "b": "gnd"}, "resistance": value(bottom, "ohm")},
        ],
    }


MODEL = curves(netlist(), SETS.read_text())


def console(raws: dict[int, int], mvs: dict[int, int] | None = None, done: bool = True) -> list[str]:
    """64 samples at each attenuation, each its raw code and IDF's millivolts of it."""
    out = ["I (10) boot: noise", "adc ready"]
    for atten in range(4):
        out.append(f"cali {atten} ok")
        raw = raws[atten]
        mv = MODEL[atten].millivolts(raw) if mvs is None else mvs[atten]
        out += [f"adc {atten} {k} {raw} {mv}" for k in range(64)]
    if done:
        out.append("ADC DONE")
    return out


def model_raws(volts: float) -> dict[int, int]:
    return {atten: MODEL[atten].code(volts * 1000) for atten in range(4)}


def raises(fn) -> bool:
    try:
        fn()
    except ValueError:
        return True
    return False


def test_the_synthetic_set_is_the_nominal_calibration() -> None:
    assert [digi(SYNTHETIC, 0, atten) for atten in range(4)] == [3200, 2400, 1700, 900]
    assert [digi(SYNTHETIC, 1, atten) for atten in range(4)] == [3240, 2410, 1720, 915]
    # The same values the chip crate's tests work out.
    assert MODEL[3].coeff_a == 61895
    assert MODEL[3].millivolts(2048) == 1934 - 39
    assert [MODEL[atten].millivolts(4095) for atten in range(3)] == [1083, 1438, 1992]


def test_the_divider_holds_half_the_supply_and_a_stronger_bottom_less() -> None:
    assert pad_volts(netlist()) == pytest.approx(1.65)
    assert pad_volts(netlist(bottom=5000.0)) == pytest.approx(1.1, abs=1e-4)


def test_the_codes_at_half_the_supply_clip_below_6_db() -> None:
    raws = model_raws(1.65)
    assert raws[0] == raws[1] == 4095
    assert raws[2] < 4095 and raws[3] < 4095
    assert abs(MODEL[2].millivolts(raws[2]) - 1650) <= 2
    assert abs(MODEL[3].millivolts(raws[3]) - 1650) <= 2


def test_the_models_codes_pass_and_a_code_off_by_two_fails() -> None:
    raws = model_raws(1.65)
    assert not raises(lambda: check_probe(Probe(console(raws)), MODEL, 1.65))
    for atten in (2, 3):
        assert not raises(lambda: check_probe(Probe(console({**raws, atten: raws[atten] + 1})), MODEL, 1.65))
        assert raises(lambda: check_probe(Probe(console({**raws, atten: raws[atten] + 2})), MODEL, 1.65))


def test_a_wrong_attenuation_scale_fails() -> None:
    raws = model_raws(1.65)
    # 11 dB read on 6 dB's curve.
    wrong = {**raws, 3: MODEL[2].code(1650)}
    assert raises(lambda: check_probe(Probe(console(wrong)), MODEL, 1.65))
    # A clipping attenuation that reads below the top.
    assert raises(lambda: check_probe(Probe(console({**raws, 1: 4000})), MODEL, 1.65))


def test_millivolts_must_be_idfs_and_within_the_datasheets_error() -> None:
    raws = model_raws(1.65)
    mvs = {atten: MODEL[atten].millivolts(raws[atten]) for atten in range(4)}
    assert raises(lambda: check_probe(Probe(console(raws, {**mvs, 3: mvs[3] + 1})), MODEL, 1.65))
    # Right calibration, wrong voltage: 1.7 V's codes against a 1.65 V pad.
    far = model_raws(1.70)
    assert raises(lambda: check_probe(Probe(console(far)), MODEL, 1.65))


def test_a_console_without_its_lines_fails() -> None:
    raws = model_raws(1.65)
    assert raises(lambda: Probe(console(raws, done=False)))
    text = [line for line in console(raws) if line != "adc 2 63 3364 1650"]
    assert raises(lambda: Probe([line for line in text if not line.startswith("adc 2 63 ")]))
    failed = [line.replace("cali 3 ok", "cali 3 ESP_ERR_NOT_SUPPORTED") for line in console(raws)]
    assert raises(lambda: Probe(failed))


def test_a_set_without_calibration_has_no_curve() -> None:
    fields = {**SYNTHETIC, "BLK_VERSION_MAJOR": 0}
    assert raises(lambda: Curve(fields, 0, 3))
    assert raises(lambda: efuse_set("no such set", SETS.read_text()))


def test_a_board_that_reads_what_the_model_would_passes_and_one_that_does_not_fails() -> None:
    raws = model_raws(1.62)
    board = Probe(console(raws))
    assert not raises(lambda: check_board(board, MODEL))
    off = {**raws, 3: raws[3] + 40}
    mvs = {atten: MODEL[atten].millivolts(raws[atten]) for atten in range(4)}
    assert raises(lambda: check_board(Probe(console(off, mvs)), MODEL))


def test_a_whole_run(tmp_path: Path) -> None:
    run = tmp_path / "run"
    (run / "traces").mkdir(parents=True)
    net = netlist()
    (run / "netlist.json").write_text(json.dumps(net))
    rows = ["t_ps,part,usart,byte"]
    text = "\r\n".join(console(model_raws(1.65))) + "\r\n"
    rows += [f"1,u1,USB_SERIAL_JTAG,{ord(c)}" for c in text]
    (run / "traces" / "serial.csv").write_text("\n".join(rows) + "\n")
    (run / "diagnostics.json").write_text(json.dumps([{"code": "chip.unmodeled_register", "severity": "warning"}]))
    blobs = [{"role": "rom", "sha256": "r", "bytes": 1}, {"role": "flash", "sha256": "f", "bytes": 2, "offset": 8}]
    (run / "manifest.json").write_text(json.dumps({"blobs": blobs}))
    report = check(run, example=tmp_path)
    assert "no board.txt" in report
    (run / "diagnostics.json").write_text(json.dumps([{"code": "chip.unsupported_setting", "severity": "error"}]))
    assert raises(lambda: check(run, example=tmp_path))
