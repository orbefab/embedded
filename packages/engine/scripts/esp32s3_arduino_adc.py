# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of the adc sketch, apart from the engine.

    uv run packages/engine/scripts/esp32s3_arduino_adc.py --check RUN_DIR

The sketch prints `adc <k> raw <raw> mv <mv>` eight times, `analogRead(6)` and
`analogReadMilliVolts(6)` at Arduino-ESP32 3.3.12's default attenuation and width, 11 dB and 12
bits (`cores/esp32/esp32-hal-adc.c`), then `ADC DONE`. Both go through IDF's one-shot driver, the
second through its curve-fitting calibration, so each line must be the IDF probe's 11 dB line
for the same circuit: the raw code within 1 of the closed form `esp32s3_adc` checks the IDF
probe against, and the millivolts IDF's calibration of that raw code.

`traces/serial.csv` must hold those lines as `u1`'s `USB_SERIAL_JTAG`. Other lines may sit
between them. The run must hold no error, and its `manifest.json` must list the netlist's ROM,
then each flash region with its offset. The netlist is
`packages/engine/tests/firmware/esp32s3/arduino/checks/adc/netlist.json`, the circuit of
`packages/engine/tests/circuits/esp32s3-adc`.
"""

import argparse
import json
import re
import sys
from pathlib import Path

from esp32s3_adc import SETS, check_diagnostics, check_manifest, curves, lines, pad_volts

ATTEN = 3
PASSES = 8
LINE = re.compile(r"adc (\d+) raw (\d+) mv (\d+)")


def readings(text: list[str]) -> list[tuple[int, int]]:
    """Each pass's raw code and millivolts, in order, through `ADC DONE`."""
    out = []
    for line in text:
        line = line.rstrip("\r")
        if line == "ADC DONE":
            if len(out) != PASSES:
                raise ValueError(f"the sketch printed {len(out)} readings, not {PASSES}")
            return out
        if match := LINE.fullmatch(line):
            k, raw, mv = (int(group) for group in match.groups())
            if k != len(out):
                raise ValueError(f"reading {k} is out of order")
            out.append((raw, mv))
    raise ValueError("the console has no ADC DONE line")


def check_readings(found: list[tuple[int, int]], netlist: dict, sets_text: str) -> str:
    volts = pad_volts(netlist)
    curve = curves(netlist, sets_text)[ATTEN]
    want = curve.code(volts * 1000)
    for raw, mv in found:
        if abs(raw - want) > 1:
            raise ValueError(f"analogRead reads {raw}, not the 11 dB line's {want} within 1")
        if mv != curve.millivolts(raw):
            raise ValueError(f"analogReadMilliVolts reads {mv}, not IDF's {curve.millivolts(raw)} for {raw}")
    raws = sorted({raw for raw, _ in found})
    mvs = sorted({mv for _, mv in found})
    return f"GPIO6 at {volts * 1000:.1f} mV: analogRead {raws} against {want}, analogReadMilliVolts {mvs}"


def check(run: Path, sets: Path = SETS) -> str:
    netlist = json.loads((run / "netlist.json").read_text())
    found = readings(lines((run / "traces" / "serial.csv").read_text()))
    clause = check_readings(found, netlist, sets.read_text())
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    report = f"esp32s3-arduino-adc: {clause}"
    print(report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"esp32s3-arduino-adc: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
