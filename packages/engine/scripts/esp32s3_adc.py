# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check a run of `packages/engine/tests/circuits/esp32s3-adc` against the ADC's closed form, apart from the engine.

    uv run packages/engine/scripts/esp32s3_adc.py --check RUN_DIR

GPIO6, ADC1 channel 5, sits on a divider. After `g` the app prints, for each attenuation 0 to 3
(0, 2.5, 6 and 11 dB), `cali <atten> ok` and 64 lines `adc <atten> <k> <raw> <mv>`: the raw
code of IDF's one-shot read and the millivolts of IDF's curve-fitting calibration, then
`ADC DONE`.

The pad's voltage is the netlist's: the divider's resistors to nets a voltage source holds, and
the part's leakage to `VCC` and to `GND`, solved for the one node. IDF's oneshot driver turns the
pad's pulls off. The closed form takes the eFuse set the part names, `synthetic` by default, from
`packages/socs/espressif/src/s3/efuse.txt`, and IDF v5.5.5's sums and coefficients:
`components/efuse/esp32s3/esp_efuse_rtc_calib.c`, `components/esp_adc/adc_cali_curve_fitting.c`
and `components/esp_adc/esp32s3/curve_fitting_coefficients.c`. The calibration's two steps in
real numbers, inverted, rounded to the nearest code and clipped to 0 and 4095, give the code
for the voltage.

Each attenuation's 64 raw codes must lie within 1 of that code. Each millivolt reading must be
IDF's integer calibration of its raw code, and where the code does not clip, within the total
error the datasheet states after calibration of the pad's voltage (Espressif ESP32-S3 Series
Datasheet v2.2, Table 5-6: 5, 6, 10 and 50 mV). The attenuations that do not clip must agree with
each other within the sum of their errors. One that clips must read 4095.

When `board.txt` sits beside the netlist, it holds a board's console from `adc ready` through
`ADC DONE`, from a board whose eFuse set the netlist names. For every sample, at every
attenuation and including a clipped raw of 4095, the model's `millivolts` of that raw code must
equal the board's printed millivolts. The board's firmware printed that column from IDF's
`adc_cali_raw_to_voltage` with the same eFuses. The board's pin voltage is not compared, because
`VCC_OUT` has no sourced voltage. The 3.3 V run and the board's pin are not the same input. The
board tests the eFuse curve. The init codes are not testable from the board, because firmware
writes the same eFuse the model checks against. With no `board.txt` that comparison is skipped,
and the check says so.

`traces/serial.csv` must hold those lines as `u1`'s `USB_SERIAL_JTAG`. A `UART0`, `UART1` or
`UART2` row may share the file: a flash boot programs UART0, and the early log is sent there
before the console moves. Other lines may sit between the console lines. The run must hold no
error, and its `manifest.json` must list the netlist's ROM,
then each flash region with its offset.
"""

import argparse
import csv
import io
import json
import re
import statistics
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EXAMPLE = ROOT / "packages/engine/tests/circuits" / "esp32s3-adc"
SETS = ROOT / "packages/socs/espressif/src/s3/efuse.txt"
PIN = "GPIO6"
UNIT = 0
SAMPLES = 64
ATTENS = range(4)
MAX = 4095
# Table 5-6, total error after calibration, by attenuation.
ERROR_MV = (5, 6, 10, 50)
CALI = re.compile(r"cali ([0-3]) (\S+)")
ADC = re.compile(r"adc ([0-3]) (\d+) (-?\d+) (-?\d+)")

E16, E15 = 10**16, 10**15
# By unit and attenuation, each term's coefficient, divisor and sign.
TERMS = (
    (
        ((27856531419538344, E16, -1), (50871540569528, E16, -1), (9798249589, E15, 1)),
        ((29831022915028695, E16, -1), (49393185868806, E16, -1), (101379430548, E16, 1)),
        ((23285545746296417, E16, -1), (147640181047414, E16, -1), (208385525314, E16, 1)),
        (
            (644403418269478, E15, -1),
            (644334888647536, E16, -1),
            (1297891447611, E16, 1),
            (70769718, E15, -1),
            (13515, E15, 1),
        ),
    ),
    (
        ((25668651654328927, E16, -1), (1353548869615, E16, 1), (36615265189, E16, 1)),
        ((23690184690298404, E16, -1), (66319894226185, E16, -1), (118964995959, E16, 1)),
        ((9452499397020617, E16, -1), (200996773954387, E16, -1), (259011467956, E16, 1)),
        (
            (12247719764336924, E16, 1),
            (755717904943462, E16, -1),
            (1478791187119, E16, 1),
            (79672528, E15, -1),
            (15038, E15, 1),
        ),
    ),
)
# Each calibration field's place: block, first bit, length.
FIELDS = {
    "BLK_VERSION_MAJOR": (2, 128, 2),
    "ADC1_CAL_VOL_ATTEN0": (2, 201, 8),
    "ADC1_CAL_VOL_ATTEN1": (2, 209, 8),
    "ADC1_CAL_VOL_ATTEN2": (2, 217, 8),
    "ADC1_CAL_VOL_ATTEN3": (2, 225, 8),
    "ADC2_CAL_VOL_ATTEN0": (2, 233, 8),
    "ADC2_CAL_VOL_ATTEN1": (2, 241, 7),
    "ADC2_CAL_VOL_ATTEN2": (2, 248, 7),
    "ADC2_CAL_VOL_ATTEN3": (1, 186, 6),
}
U32 = (1 << 32) - 1
U64 = (1 << 64) - 1


def efuse_set(name: str, text: str) -> dict[str, int]:
    """The calibration fields of set `name` in `efuse.txt`, each a raw unsigned value; a field the
    set does not name is 0."""
    sets: dict[str, dict[str, int]] = {}
    current = None
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            current = sets.setdefault(line[1:-1], {})
            continue
        if current is None:
            raise ValueError("efuse.txt names a field before a set")
        field, value = line.split(None, 1)
        if field in FIELDS:
            current[field] = int(value, 0)
    if name not in sets:
        raise ValueError(f"efuse.txt has no set {name!r}")
    return {field: sets[name].get(field, 0) for field in FIELDS}


def digi(fields: dict[str, int], unit: int, atten: int) -> int:
    """`esp_efuse_rtc_calib_get_cal_voltage`'s code at 850 mV, in unsigned 32-bit sums."""
    d1 = [fields[f"ADC1_CAL_VOL_ATTEN{k}"] for k in range(4)]
    d2 = [fields[f"ADC2_CAL_VOL_ATTEN{k}"] for k in range(4)]
    vol = [0] * 4
    vol[3] = (d1[3] + 900) & U32
    vol[2] = (d1[2] + vol[3] + 800) & U32
    vol[1] = (d1[1] + vol[2] + 700) & U32
    vol[0] = (d1[0] + vol[1] + 800) & U32
    if unit == 0:
        return vol[atten]
    return (vol[atten] - d2[atten] + (40, 10, 20, 15)[atten]) & U32


class Curve:
    """One unit's calibration at one attenuation."""

    def __init__(self, fields: dict[str, int], unit: int, atten: int) -> None:
        if fields["BLK_VERSION_MAJOR"] != 1:
            raise ValueError("the eFuse set has no ADC calibration IDF reads")
        self.coeff_a = (65536 * 850 // digi(fields, unit, atten)) & U32
        self.terms = TERMS[unit][atten]

    def millivolts(self, raw: int) -> int:
        """`adc_cali_raw_to_voltage`, with IDF's 64-bit and 32-bit wrap."""

        def i32(x: int) -> int:
            x &= U32
            return x - (1 << 32) if x >> 31 else x

        v = raw * self.coeff_a // 65536
        if v == 0:
            return 0
        k0, d0, s0 = self.terms[0]
        error = i32(k0 // d0 * s0)
        variable = 1
        for k, d, sign in self.terms[1:]:
            variable = (variable * v) & U64
            term = ((variable * k) & U64) // d
            error = i32(error + i32(term) * sign)
        return i32(v - error)

    def real(self, x: float) -> float:
        v = x * self.coeff_a / 65536
        k0, d0, s0 = self.terms[0]
        error = float(k0 // d0 * s0)
        for i, (k, d, sign) in enumerate(self.terms[1:], start=1):
            error += sign * (k / d) * v**i
        return v - error

    def code(self, mv: float) -> int:
        """The real-valued calibration inverted, rounded to the nearest code and clipped."""
        if not mv > self.real(0.0):
            return 0
        if mv >= self.real(MAX):
            return MAX
        low, high = 0.0, float(MAX)
        for _ in range(64):
            mid = (low + high) / 2
            if self.real(mid) < mv:
                low = mid
            else:
                high = mid
        return min(MAX, max(0, int((low + high) / 2 + 0.5)))


def lines(serial_csv: str) -> list[str]:
    """Each console line, without its line ending. UART rows are the boot log, not the console."""
    rows = list(csv.DictReader(io.StringIO(serial_csv)))
    known = {"USB_SERIAL_JTAG", "UART0", "UART1", "UART2"}
    if any(row["part"] != "u1" or row["usart"] not in known for row in rows):
        raise ValueError("serial.csv holds a byte that is not u1's USB_SERIAL_JTAG or a UART")
    rows = [row for row in rows if row["usart"] == "USB_SERIAL_JTAG"]
    return "".join(chr(int(row["byte"])) for row in rows).splitlines()


class Probe:
    """Each attenuation's calibration result and its samples, each a raw code and millivolts."""

    def __init__(self, text: list[str]) -> None:
        self.cali: dict[int, str] = {}
        self.samples: dict[int, list[tuple[int, int]]] = {atten: [] for atten in ATTENS}
        done = False
        for line in text:
            line = line.rstrip("\r")
            if done:
                continue
            if line == "ADC DONE":
                done = True
            elif match := CALI.fullmatch(line):
                self.cali[int(match.group(1))] = match.group(2)
            elif match := ADC.fullmatch(line):
                atten, k, raw, mv = (int(group) for group in match.groups())
                if k != len(self.samples[atten]):
                    raise ValueError(f"sample {k} at attenuation {atten} is out of order")
                self.samples[atten].append((raw, mv))
        if not done:
            raise ValueError("the console has no ADC DONE line")
        for atten in ATTENS:
            if self.cali.get(atten) != "ok":
                raise ValueError(f"the calibration at attenuation {atten} is {self.cali.get(atten)}")
            if len(self.samples[atten]) != SAMPLES:
                raise ValueError(f"attenuation {atten} has {len(self.samples[atten])} samples, not {SAMPLES}")


def pad_volts(netlist: dict) -> float:
    """The voltage on `PIN`'s net: resistors to nets a voltage source holds against ground, and
    the part's leakage to `VCC` and to `GND`."""
    ground = netlist["ground"]
    held = {ground: 0.0}
    for part in netlist["parts"]:
        if part["kind"] == "voltage_source":
            pins = part["pins"]
            if pins["n"] != ground:
                raise ValueError("the check takes voltage sources against ground")
            held[pins["p"]] = float(part["voltage"]["value"])
    chip = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    if PIN not in chip["pins"]:
        raise ValueError(f"the part has no net on {PIN}")
    node = chip["pins"][PIN]
    leak = 1 / float(chip["leakage_resistance"]["value"])
    conductances = [(leak, chip["pins"]["VCC"]), (leak, chip["pins"]["GND"])]
    for part in netlist["parts"]:
        if part["kind"] != "resistor" or node not in part["pins"].values():
            continue
        a, b = part["pins"]["a"], part["pins"]["b"]
        other = b if a == node else a
        if other not in held:
            raise ValueError(f"{part['name']} goes from {PIN} to {other}, which no source holds")
        conductances.append((1 / float(part["resistance"]["value"]), other))
    for _, net in conductances:
        if net not in held:
            raise ValueError(f"the check takes the part's supply from a source, not {net}")
    total = sum(g for g, _ in conductances)
    return sum(g * held[net] for g, net in conductances) / total


def curves(netlist: dict, sets_text: str) -> list[Curve]:
    chip = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    fields = efuse_set(chip.get("efuse", "synthetic"), sets_text)
    return [Curve(fields, UNIT, atten) for atten in ATTENS]


def check_probe(probe: Probe, curves: list[Curve], volts: float) -> list[str]:
    """Each attenuation's clause. Raises on the first that fails."""
    mv = volts * 1000
    clauses = []
    readings: dict[int, int] = {}
    for atten in ATTENS:
        curve = curves[atten]
        want = curve.code(mv)
        for raw, read in probe.samples[atten]:
            if abs(raw - want) > 1:
                raise ValueError(f"attenuation {atten} reads {raw}, not {want} within 1")
            if read != curve.millivolts(raw):
                raise ValueError(f"attenuation {atten} calibrates {raw} to {read} mV, not {curve.millivolts(raw)}")
        raws = sorted({raw for raw, _ in probe.samples[atten]})
        if want == MAX:
            if raws != [MAX]:
                raise ValueError(f"attenuation {atten} clips at {mv:.1f} mV, and reads {raws}")
            clauses.append(f"{atten}: clips at {MAX}")
            continue
        reads = [read for _, read in probe.samples[atten]]
        for read in reads:
            if abs(read - mv) > ERROR_MV[atten]:
                raise ValueError(f"attenuation {atten} reads {read} mV, not {mv:.1f} within {ERROR_MV[atten]}")
        readings[atten] = round(statistics.median(reads))
        clauses.append(f"{atten}: raw {raws} against {want}, {readings[atten]} mV")
    if not readings:
        raise ValueError(f"every attenuation clips at {mv:.1f} mV")
    for a in readings:
        for b in readings:
            if a < b and abs(readings[a] - readings[b]) > ERROR_MV[a] + ERROR_MV[b]:
                raise ValueError(f"attenuations {a} and {b} read {readings[a]} and {readings[b]} mV")
    return clauses


def check_board(board: Probe, curves: list[Curve]) -> str:
    """Each printed millivolt is the model's of that raw code, clipped samples included.

    The board's pin voltage is not compared with the run. `VCC_OUT` has no sourced voltage, so
    the 3.3 V run and the board's pin are not the same input. The board tests the eFuse curve.
    The init codes are not testable here, because firmware writes the same eFuse the model
    checks against.
    """
    samples = 0
    clipped = 0
    for atten in ATTENS:
        curve = curves[atten]
        for raw, read in board.samples[atten]:
            want = curve.millivolts(raw)
            if read != want:
                raise ValueError(f"attenuation {atten} calibrates raw {raw} to {read} mV, not {want}")
            samples += 1
            if raw == MAX:
                clipped += 1
    clause = f"the board's {samples} samples match the eFuse curve"
    if clipped:
        clause += f", {clipped} of them clipped at {MAX}"
    return clause


def check_manifest(manifest: dict, netlist: dict) -> None:
    part = next(part for part in netlist["parts"] if part["kind"] == "esp32s3")
    want = [{"role": "rom", **part["rom"]}]
    want += [
        {"role": "flash", "sha256": region["sha256"], "bytes": region["bytes"], "offset": region["offset"]}
        for region in part["firmware"]["flash"]
    ]
    if manifest["blobs"] != want:
        raise ValueError(f"manifest.json lists {manifest['blobs']}, not {want}")


def check_diagnostics(diagnostics: list[dict]) -> None:
    errors = [item["code"] for item in diagnostics if item["severity"] == "error"]
    if errors:
        raise ValueError(f"the run holds errors {errors}")


def check(run: Path, example: Path = EXAMPLE, sets: Path = SETS) -> str:
    netlist = json.loads((run / "netlist.json").read_text())
    probe = Probe(lines((run / "traces" / "serial.csv").read_text()))
    volts = pad_volts(netlist)
    model = curves(netlist, sets.read_text())
    clauses = check_probe(probe, model, volts)
    check_diagnostics(json.loads((run / "diagnostics.json").read_text()))
    check_manifest(json.loads((run / "manifest.json").read_text()), netlist)
    board_txt = example / "board.txt"
    if board_txt.exists():
        board = ", " + check_board(Probe(board_txt.read_text().splitlines()), model)
    else:
        board = ", no board.txt, so no board comparison"
    report = f"esp32s3-adc: {PIN} at {volts * 1000:.1f} mV, " + "; ".join(clauses) + board
    print(report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", type=Path, metavar="RUN_DIR", required=True)
    args = parser.parse_args()
    try:
        check(args.check)
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"esp32s3-adc: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
