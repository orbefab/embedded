# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Generate and check the catalog ngspice sweep under packages/engine/tests/circuit/catalog/.

    uv run packages/engine/scripts/catalog_sweep.py            # write netlists and ngspice references
    uv run packages/engine/scripts/catalog_sweep.py --check    # verify coverage and netlists match parts/

Reads every parts/*/part.jsonc, extracts every ngspice-translatable electrical
parameter set, builds one or two canonical circuits per unique parameter set,
and verifies that every part in parts/ is either swept or listed in EXCLUSIONS
with a reason. `ngspice_refs.py --check` verifies the committed `deck.cir` and
`ngspice.json` files for every catalog circuit.
"""

import json
import re
import shutil
import sys
from pathlib import Path

import ngspice_refs as refs

ROOT = Path(__file__).resolve().parents[3]
PARTS_DIR = ROOT / "parts"
CATALOG_DIR = ROOT / "packages/engine/tests/circuit/catalog"

# Parts that stay out of the ngspice catalog sweep, with the reason written next to each entry.
EXCLUSIONS = {
    # Op-amps: behavioural macromodel checked against datasheet bands and closed forms in runs/op_amp.rs.
    "lm358-dip8": "op_amp macromodel checked against datasheet bands and closed forms in runs/op_amp.rs",
    "lmv358-msop8": "op_amp macromodel checked against datasheet bands and closed forms in runs/op_amp.rs",
    # Analog temperature sensor and world-driven curves: ngspice has no world-schedule curve model.
    "tmp36": "analog_temp_sensor uses world temperature curves, which ngspice has no model for",
    "ldr-5mm": "resistor uses a world light curve, which ngspice has no model for",
    "thermistor-ntc-10k": "resistor uses a world temperature curve, which ngspice has no model for",
    # LEDs whose fitted saturation current falls below ngspice-42's 1e-28 A diode clamp.
    "led-5mm-green": "fitted saturation_current (6.776e-64 A) is below ngspice-42's 1e-28 A diode clamp",
    "led-5mm-yellow": "fitted saturation_current (1.779e-33 A) is below ngspice-42's 1e-28 A diode clamp",
    "led-3mm-yellow": "fitted saturation_current (1.623e-30 A) is below ngspice-42's 1e-28 A diode clamp",
    # Firmware MCUs and digital/protocol behavioral devices: no SPICE analog equivalent.
    "atmega328p-dip28": "atmega328p runs firmware, which a SPICE deck cannot",
    "atmega328p-tqfp32": "atmega328p runs firmware, which a SPICE deck cannot",
    "esp32s3-qfn56": "esp32s3 runs firmware, which a SPICE deck cannot",
    "bmp280": "digital I2C/SPI sensor behavioral model, not an analog SPICE device",
    "dht22": "digital single-wire sensor behavioral model, not an analog SPICE device",
    "hc595-dip16": "digital shift-register behavioral model, not an analog SPICE device",
    "hd44780-1602": "digital LCD controller behavioral model, not an analog SPICE device",
    "hd44780-1602-i2c": "digital I2C LCD controller behavioral model, not an analog SPICE device",
    "max7219-8x8": "digital LED-driver behavioral model, not an analog SPICE device",
    "servo-sg90": "digital PWM servo behavioral model, not an analog SPICE device",
    "sk68xxmini-hs": "digital addressable LED behavioral model, not an analog SPICE device",
    "ssd1306-oled-128x64": "digital I2C OLED controller behavioral model, not an analog SPICE device",
    "ws2812b-5050": "digital addressable LED behavioral model, not an analog SPICE device",
    # Host USB/jack stub sources: idle voltage sources whose primitive is exercised by every sweep circuit.
    "atmega16u2-qfn32": "USB-serial host stub (idle serial/steps voltage sources only)",
    "barrel-jack-dc21": "parameterized supply voltage source stub",
    "cp2102n-qfn28": "USB-serial host stub (idle serial/steps voltage sources only)",
    "usb-b-socket": "fixed 5 V USB supply voltage source stub",
    "usb-micro-b": "fixed 5 V USB supply voltage source stub",
    "usb-mini-b": "fixed 5 V USB supply voltage source stub",
}


def strip_jsonc(text):
    out = []
    i = 0
    in_str = False
    n = len(text)
    while i < n:
        ch = text[i]
        if in_str:
            out.append(ch)
            if ch == "\\" and i + 1 < n:
                out.append(text[i + 1])
                i += 2
                continue
            if ch == '"':
                in_str = False
            i += 1
        elif ch == '"':
            in_str = True
            out.append(ch)
            i += 1
        elif text[i : i + 2] == "//":
            nl = text.find("\n", i)
            i = n if nl == -1 else nl
        elif text[i : i + 2] == "/*":
            end = text.find("*/", i + 2)
            i = n if end == -1 else end + 2
        else:
            out.append(ch)
            i += 1
    return json.loads(re.sub(r",(\s*[}\]])", r"\1", "".join(out)))


def strip_sources(obj):
    if isinstance(obj, dict):
        return {k: strip_sources(v) for k, v in obj.items() if k != "source"}
    if isinstance(obj, list):
        return [strip_sources(v) for v in obj]
    return obj


def q(value, unit):
    return {"value": value, "unit": unit}


def vsrc(name, p, n, voltage):
    return {"name": name, "kind": "voltage_source", "pins": {"p": p, "n": n}, "voltage": voltage}


def resistor(name, a, b, ohms):
    return {
        "name": name,
        "kind": "resistor",
        "pins": {"a": a, "b": b},
        "resistance": q(ohms, "ohm"),
        "ratings": {"power": None},
    }


def step_v(initial, final, at_ps):
    return {
        "kind": "step",
        "initial": q(initial, "V"),
        "final": q(final, "V"),
        "at_ps": at_ps,
    }


def pulse_v(initial, pulsed, delay_ps, width_ps, period_ps):
    return {
        "kind": "pulse",
        "initial": q(initial, "V"),
        "pulsed": q(pulsed, "V"),
        "delay_ps": delay_ps,
        "width_ps": width_ps,
        "period_ps": period_ps,
    }


def op_netlist(nets, parts):
    return {
        "version": 1,
        "seed": 0,
        "analysis": {"op": {}},
        "ground": "gnd",
        "nets": nets,
        "parts": parts,
    }


def tran_netlist(stop_ps, max_step_ps, grid_ps, nets, parts):
    return {
        "version": 1,
        "seed": 0,
        "analysis": {
            "transient": {
                "stop_ps": stop_ps,
                "max_step_ps": max_step_ps,
                "grid_ps": grid_ps,
            }
        },
        "ground": "gnd",
        "nets": nets,
        "parts": parts,
    }


def extract_part_entries():
    """Return (entries, accounted_parts) from parts/*/part.jsonc.

    Each entry is (id_name, kind, electrical_dict, part_name).
    Every directory under parts/ must either produce an entry, be structural
    (joins_only, not_simulated without model, or a non-battery composite whose
    leaf parts are swept), or be listed in EXCLUSIONS.
    """
    all_parts = sorted(p.parent.name for p in PARTS_DIR.glob("*/part.jsonc"))
    unknown_exclusions = sorted(set(EXCLUSIONS) - set(all_parts))
    if unknown_exclusions:
        raise SystemExit(f"catalog_sweep: EXCLUSIONS names unknown parts: {unknown_exclusions}")

    entries = []
    unaccounted = []

    for name in all_parts:
        if name in EXCLUSIONS:
            continue
        data = strip_jsonc((PARTS_DIR / name / "part.jsonc").read_text())

        # Structural join-only or non-simulated parts with no electrical model.
        if data.get("joins_only") or (data.get("not_simulated") and "model" not in data and "parts" not in data):
            continue

        # Battery composites: voltage_source cell + series internal resistance.
        if name in ("battery-9v", "battery-aa-4x"):
            cell_part = strip_jsonc((PARTS_DIR / f"{name}-cell" / "part.jsonc").read_text())
            voltage = float(cell_part["model"]["voltage"]["value"])
            r_int = float(data["parts"]["r_int"]["resistance"]["value"])
            entries.append((name, "battery", {"voltage": voltage, "r_int": r_int}, name))
            continue

        # Leaf battery cells are covered by their battery composites above.
        if name in ("battery-9v-cell", "battery-aa-4x-cell"):
            continue

        # Non-battery composite assemblies/boards: every translatable leaf part they use is swept directly.
        if "model" not in data and "parts" in data:
            continue

        model = data.get("model")
        if not isinstance(model, dict):
            unaccounted.append(name)
            continue

        if "kind" in model:
            kind = model["kind"]
            params = data.get("params") or {}
            extracted = extract_single_model(name, kind, model, params)
            if not extracted:
                unaccounted.append(name)
            else:
                entries.extend(extracted)
        else:
            # Multi-model package (e.g., ch340g, display-7seg-*, led-5mm-rgb-*, resistor-array-4x0603, resonator-cstce).
            found_any = False
            for sub_name, sub_model in sorted(model.items()):
                if not isinstance(sub_model, dict) or "kind" not in sub_model:
                    continue
                sub_kind = sub_model["kind"]
                if sub_kind == "voltage_source":
                    # Host TX/DTR stubs inside ch340g.
                    continue
                extracted = extract_single_model(f"{name}-{sub_name}", sub_kind, sub_model, data.get("params") or {})
                if extracted:
                    for eid, ekind, efields, _ in extracted:
                        entries.append((eid, ekind, efields, name))
                    found_any = True
            if not found_any:
                unaccounted.append(name)

    if unaccounted:
        raise SystemExit(f"catalog_sweep: parts neither swept nor in EXCLUSIONS: {unaccounted}")
    return entries


def extract_single_model(label, kind, model, params):
    clean = strip_sources(model)
    clean.pop("kind", None)
    clean.pop("pins", None)
    clean.pop("ratings", None)

    if kind == "led" and "color" in params and "choices" in params["color"]:
        out = []
        for color, spec in sorted(params["color"]["choices"].items()):
            merged = dict(clean)
            for pointer, val in spec.get("sets", {}).items():
                merged[pointer.lstrip("/")] = strip_sources(val)
            out.append((f"{label}-{color}", kind, merged, label))
        return out

    if kind == "potentiometer":
        if "resistance" not in clean:
            clean["resistance"] = q(float(params["resistance"]["example"]), "ohm")
        clean.pop("position", None)
        return [(label, kind, clean, label)]

    if kind == "inductor":
        if "inductance" not in clean:
            clean["inductance"] = q(float(params["inductance"]["example"]), "H")
        if "series_resistance" not in clean:
            clean["series_resistance"] = q(float(params["series_resistance"]["example"]), "ohm")
        return [(label, kind, clean, label)]

    if kind == "capacitor":
        if "capacitance" not in clean:
            clean["capacitance"] = q(float(params["capacitance"]["example"]), "F")
        clean["initial"] = None
        return [(label, kind, clean, label)]

    if kind == "resistor":
        if "resistance" not in clean:
            spec = params.get("resistance") or next(
                (v for v in params.values() if isinstance(v, dict) and str(v.get("sets", "")).endswith("/resistance")),
                None,
            )
            clean["resistance"] = q(float(spec["example"]), "ohm")
        return [(label, kind, clean, label)]

    if kind in ("diode", "led", "zener", "npn", "pnp", "nmos", "pmos", "linear_regulator", "relay", "switch"):
        return [(label, kind, clean, label)]

    return []


def normalize_numbers(obj):
    if isinstance(obj, dict):
        return {k: normalize_numbers(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return [normalize_numbers(v) for v in obj]
    if isinstance(obj, int) and not isinstance(obj, bool):
        return float(obj)
    return obj


def dedup_entries(entries):
    """Keep one canonical entry per unique (kind, electrical_params) signature."""
    seen = {}
    unique = []
    for eid, kind, fields, part_name in entries:
        key = (kind, json.dumps(normalize_numbers(fields), sort_keys=True))
        if key not in seen:
            seen[key] = [eid, kind, fields, [part_name]]
            unique.append(seen[key])
        elif part_name not in seen[key][3]:
            seen[key][3].append(part_name)
    return unique


def circuits_for_entry(eid, kind, fields):
    """Build 1-2 canonical circuits for one unique catalog parameter set."""
    if kind in ("diode", "led"):
        part_fwd = {
            "name": "d1",
            "kind": kind,
            "pins": {"anode": "va", "cathode": "gnd"},
            **fields,
            "ratings": {"current": None, "power": None},
        }
        part_rev = {
            "name": "d1",
            "kind": kind,
            "pins": {"anode": "va", "cathode": "gnd"},
            **fields,
            "ratings": {"current": None, "power": None},
        }
        return {
            f"{eid}-fwd": op_netlist(
                ["gnd", "vin", "va"],
                [vsrc("v1", "vin", "gnd", q(5.0, "V")), resistor("r1", "vin", "va", 220.0), part_fwd],
            ),
            f"{eid}-rev": op_netlist(
                ["gnd", "vin", "va"],
                [vsrc("v1", "vin", "gnd", q(-3.0, "V")), resistor("r1", "vin", "va", 10000.0), part_rev],
            ),
        }

    if kind == "zener":
        z_brk = {
            "name": "dz1",
            "kind": "zener",
            "pins": {"anode": "gnd", "cathode": "vz"},
            **fields,
            "ratings": {"current": None, "power": None},
        }
        z_fwd = {
            "name": "dz1",
            "kind": "zener",
            "pins": {"anode": "vz", "cathode": "gnd"},
            **fields,
            "ratings": {"current": None, "power": None},
        }
        return {
            f"{eid}-breakdown": op_netlist(
                ["gnd", "vin", "vz"],
                [vsrc("v1", "vin", "gnd", q(12.0, "V")), resistor("r1", "vin", "vz", 220.0), z_brk],
            ),
            f"{eid}-fwd": op_netlist(
                ["gnd", "vin", "vz"],
                [vsrc("v1", "vin", "gnd", q(5.0, "V")), resistor("r1", "vin", "vz", 220.0), z_fwd],
            ),
        }

    if kind == "npn":
        # Choose collector load to sit in active/high-injection region (around ~150 mA for 2N2222 with ikf=0.57 A).
        rc = 22.0 if "forward_knee_current" in fields else 100.0
        rb = 4700.0 if "forward_knee_current" in fields else 22000.0
        q_part = {
            "name": "q1",
            "kind": "npn",
            "pins": {"collector": "c", "base": "b", "emitter": "gnd"},
            **fields,
            "ratings": {"current": None, "power": None},
        }
        return {
            f"{eid}-active": op_netlist(
                ["gnd", "vcc", "vin", "b", "c"],
                [
                    vsrc("vcc", "vcc", "gnd", q(5.0, "V")),
                    vsrc("vin", "vin", "gnd", q(5.0, "V")),
                    resistor("rb", "vin", "b", rb),
                    resistor("rc", "vcc", "c", rc),
                    q_part,
                ],
            ),
            f"{eid}-tran": tran_netlist(
                100_000_000,
                1_000_000,
                1_000_000,
                ["gnd", "vcc", "vin", "b", "c"],
                [
                    vsrc("vcc", "vcc", "gnd", q(5.0, "V")),
                    vsrc("vin", "vin", "gnd", pulse_v(0.0, 5.0, 5_000_000, 40_000_000, 200_000_000)),
                    resistor("rb", "vin", "b", 1_000_000.0),
                    resistor("rc", "vcc", "c", 10_000.0),
                    q_part,
                ],
            ),
        }

    if kind == "pnp":
        q_part = {
            "name": "q1",
            "kind": "pnp",
            "pins": {"collector": "c", "base": "b", "emitter": "vcc"},
            **fields,
            "ratings": {"current": None, "power": None},
        }
        return {
            f"{eid}-active": op_netlist(
                ["gnd", "vcc", "vin", "b", "c"],
                [
                    vsrc("vcc", "vcc", "gnd", q(5.0, "V")),
                    vsrc("vin", "vin", "gnd", q(0.0, "V")),
                    resistor("rb", "vin", "b", 22000.0),
                    resistor("rc", "c", "gnd", 100.0),
                    q_part,
                ],
            ),
            f"{eid}-tran": tran_netlist(
                100_000_000,
                1_000_000,
                1_000_000,
                ["gnd", "vcc", "vin", "b", "c"],
                [
                    vsrc("vcc", "vcc", "gnd", q(5.0, "V")),
                    vsrc("vin", "vin", "gnd", pulse_v(5.0, 0.0, 5_000_000, 40_000_000, 200_000_000)),
                    resistor("rb", "vin", "b", 1_000_000.0),
                    resistor("rc", "c", "gnd", 10_000.0),
                    q_part,
                ],
            ),
        }

    if kind == "nmos":
        vto = float(fields["threshold_voltage"]["value"])
        cgs = float(fields["gate_source_capacitance"]["value"])
        vdd = 10.0 if vto > 2.0 else 5.0
        vg = vto + 1.5
        rg_tran = 1_000_000.0 if cgs < 1e-10 else 100_000.0
        m_part = {
            "name": "m1",
            "kind": "nmos",
            "pins": {"drain": "d", "gate": "g", "source": "gnd"},
            **fields,
            "ratings": {"current": None, "power": None},
        }
        return {
            f"{eid}-on": op_netlist(
                ["gnd", "vdd", "vg", "g", "d"],
                [
                    vsrc("vdd", "vdd", "gnd", q(vdd, "V")),
                    vsrc("vg", "vg", "gnd", q(vg, "V")),
                    resistor("rg", "vg", "g", 1000.0),
                    resistor("rd", "vdd", "d", 100.0),
                    m_part,
                ],
            ),
            f"{eid}-tran": tran_netlist(
                200_000_000,
                2_000_000,
                2_000_000,
                ["gnd", "vdd", "vg", "g", "d"],
                [
                    vsrc("vdd", "vdd", "gnd", q(vdd, "V")),
                    vsrc("vg", "vg", "gnd", pulse_v(0.0, vdd, 10_000_000, 80_000_000, 400_000_000)),
                    resistor("rg", "vg", "g", rg_tran),
                    resistor("rd", "vdd", "d", 100.0),
                    m_part,
                ],
            ),
        }

    if kind == "pmos":
        vto = float(fields["threshold_voltage"]["value"])
        cgs = float(fields["gate_source_capacitance"]["value"])
        vdd = 10.0 if vto > 2.0 else 5.0
        vg = vdd - (vto + 1.5)
        rg_tran = 1_000_000.0 if cgs < 1e-10 else 100_000.0
        m_part = {
            "name": "m1",
            "kind": "pmos",
            "pins": {"drain": "d", "gate": "g", "source": "vdd"},
            **fields,
            "ratings": {"current": None, "power": None},
        }
        return {
            f"{eid}-on": op_netlist(
                ["gnd", "vdd", "vg", "g", "d"],
                [
                    vsrc("vdd", "vdd", "gnd", q(vdd, "V")),
                    vsrc("vg", "vg", "gnd", q(vg, "V")),
                    resistor("rg", "vg", "g", 1000.0),
                    resistor("rd", "d", "gnd", 100.0),
                    m_part,
                ],
            ),
            f"{eid}-tran": tran_netlist(
                200_000_000,
                2_000_000,
                2_000_000,
                ["gnd", "vdd", "vg", "g", "d"],
                [
                    vsrc("vdd", "vdd", "gnd", q(vdd, "V")),
                    vsrc("vg", "vg", "gnd", pulse_v(vdd, 0.0, 10_000_000, 80_000_000, 400_000_000)),
                    resistor("rg", "vg", "g", rg_tran),
                    resistor("rd", "d", "gnd", 100.0),
                    m_part,
                ],
            ),
        }

    if kind == "linear_regulator":
        vnom = float(fields["output_voltage"]["value"])
        vdo = float(fields["dropout_voltage"]["value"])
        rdo = float(fields["dropout_resistance"]["value"])
        ilim = float(fields["current_limit"]["value"])
        # Normal regulation: Vin well above Vnom + Vdo + Iload*Rdo, load at 30% of Ilim.
        iload = 0.3 * ilim
        rload = round(vnom / iload, 3)
        vin_norm = round(vnom + vdo + iload * rdo + 2.5, 3)
        # Dropout: Vin below Vnom + Vdo + Iload*Rdo but clearly above Vdo.
        vin_drop = round(max(vdo + 0.5, vnom - 0.3), 3)
        # Current limit: low load resistance that drives pass current past Ilim.
        rshort = round(vnom / (2.0 * ilim), 4)
        u_part = {
            "name": "u1",
            "kind": "linear_regulator",
            "pins": {"in": "vin", "out": "vout", "gnd": "gnd"},
            **fields,
            "ratings": {"current": None, "power": None},
        }
        return {
            f"{eid}-reg": op_netlist(
                ["gnd", "vin", "vout"],
                [vsrc("v1", "vin", "gnd", q(vin_norm, "V")), resistor("rl", "vout", "gnd", rload), u_part],
            ),
            f"{eid}-dropout": op_netlist(
                ["gnd", "vin", "vout"],
                [vsrc("v1", "vin", "gnd", q(vin_drop, "V")), resistor("rl", "vout", "gnd", rload), u_part],
            ),
            f"{eid}-limit": op_netlist(
                ["gnd", "vin", "vout"],
                [vsrc("v1", "vin", "gnd", q(vin_norm, "V")), resistor("rl", "vout", "gnd", rshort), u_part],
            ),
        }

    if kind == "potentiometer":
        out = {}
        for pos, tag in ((0.1, "p10"), (0.5, "p50"), (0.9, "p90")):
            p_part = {
                "name": "rv1",
                "kind": "potentiometer",
                "pins": {"a": "vcc", "wiper": "w", "b": "gnd"},
                **fields,
                "position": q(pos, "1"),
            }
            out[f"{eid}-{tag}"] = op_netlist(
                ["gnd", "vcc", "w"],
                [vsrc("v1", "vcc", "gnd", q(5.0, "V")), p_part, resistor("rl", "w", "gnd", 10000.0)],
            )
        return out

    if kind == "relay":
        # 5 V coil switched with a 1N4007 flyback diode and asymmetric loads on NO (1 kOhm) and NC (2.2 kOhm).
        d1n4007 = strip_sources(strip_jsonc((PARTS_DIR / "diode-1n4007/part.jsonc").read_text())["model"])
        d1n4007.pop("pins", None)
        d1n4007["ratings"] = {"current": None, "power": None}
        k_part = {
            "name": "k1",
            "kind": "relay",
            "pins": {"coil_a": "vcc", "coil_b": "coil", "com": "vcc", "no": "no", "nc": "nc"},
            **fields,
        }
        sw_part = {
            "name": "sw1",
            "kind": "switch",
            "pins": {"a": "coil", "b": "gnd"},
            "on_resistance": q(0.1, "ohm"),
            "off_resistance": q(1e8, "ohm"),
            "initial": "open",
            "events": [
                {"t_ps": 1_000_000_000, "state": "closed"},
                {"t_ps": 7_000_000_000, "state": "open"},
            ],
            "bounce": None,
        }
        return {
            f"{eid}-tran": tran_netlist(
                15_000_000_000,
                100_000_000,
                100_000_000,
                ["gnd", "vcc", "coil", "no", "nc"],
                [
                    vsrc("vcc", "vcc", "gnd", q(5.0, "V")),
                    sw_part,
                    {"name": "d1", "pins": {"anode": "coil", "cathode": "vcc"}, **d1n4007},
                    k_part,
                    resistor("rno", "no", "gnd", 1000.0),
                    resistor("rnc", "nc", "gnd", 2200.0),
                ],
            ),
        }

    if kind == "inductor":
        l_part = {
            "name": "l1",
            "kind": "inductor",
            "pins": {"a": "vin", "b": "gnd"},
            **fields,
        }
        # Drive directly (tau = L / Rs = 1 us >> 1 ns edge ramp) for transient.
        return {
            f"{eid}-op": op_netlist(
                ["gnd", "vin", "mid"],
                [
                    vsrc("v1", "vin", "gnd", q(5.0, "V")),
                    resistor("r1", "vin", "mid", 10.0),
                    {**l_part, "pins": {"a": "mid", "b": "gnd"}},
                ],
            ),
            f"{eid}-tran": tran_netlist(
                5_000_000,
                50_000,
                50_000,
                ["gnd", "vin"],
                [
                    vsrc("v1", "vin", "gnd", step_v(0.0, 1.0, 500_000)),
                    l_part,
                ],
            ),
        }

    if kind == "battery":
        v = fields["voltage"]
        r_int = fields["r_int"]
        rload = 220.0 if v >= 8.0 else 100.0
        return {
            f"{eid}-load": op_netlist(
                ["gnd", "cell", "vout"],
                [
                    vsrc("cell", "cell", "gnd", q(v, "V")),
                    resistor("r_int", "cell", "vout", r_int),
                    resistor("rl", "vout", "gnd", rload),
                ],
            ),
        }

    if kind == "capacitor":
        c_val = float(fields["capacitance"]["value"])
        # Choose R so tau = R * C is >= 10 us (well above the 1 ns edge ramp).
        r_val = 1000.0 if c_val >= 1e-6 else (1000.0 if c_val >= 1e-9 else 1_000_000.0)
        tau_ps = max(int(round(r_val * c_val * 1e12)), 1_000_000)
        stop_ps = 5 * tau_ps
        step_ps = max(stop_ps // 100, 1000)
        # Ensure grid divides stop cleanly.
        stop_ps = step_ps * 100
        c_part = {
            "name": "c1",
            "kind": "capacitor",
            "pins": {"a": "vc", "b": "gnd"},
            **fields,
        }
        return {
            f"{eid}-tran": tran_netlist(
                stop_ps,
                step_ps,
                step_ps,
                ["gnd", "vin", "vc"],
                [
                    vsrc("v1", "vin", "gnd", step_v(0.0, 5.0, step_ps * 10)),
                    resistor("r1", "vin", "vc", r_val),
                    c_part,
                ],
            ),
        }

    if kind == "switch":
        sw_part = {
            "name": "sw1",
            "kind": "switch",
            "pins": {"a": "vcc", "b": "out"},
            "on_resistance": fields["on_resistance"],
            "off_resistance": fields["off_resistance"],
            "initial": "open",
            "events": [
                {"t_ps": 1_000_000, "state": "closed"},
                {"t_ps": 5_000_000, "state": "open"},
            ],
            "bounce": None,
        }
        return {
            f"{eid}-tran": tran_netlist(
                10_000_000,
                100_000,
                100_000,
                ["gnd", "vcc", "out"],
                [
                    vsrc("v1", "vcc", "gnd", q(5.0, "V")),
                    sw_part,
                    resistor("rl", "out", "gnd", 1000.0),
                ],
            ),
        }

    if kind == "resistor":
        r_part = {
            "name": "r1",
            "kind": "resistor",
            "pins": {"a": "mid", "b": "gnd"},
            **fields,
            "ratings": {"power": None},
        }
        return {
            f"{eid}-op": op_netlist(
                ["gnd", "vin", "mid"],
                [vsrc("v1", "vin", "gnd", q(5.0, "V")), resistor("rsrc", "vin", "mid", 100.0), r_part],
            ),
        }

    raise SystemExit(f"catalog_sweep: unhandled kind {kind} for {eid}")


def build_all():
    entries = extract_part_entries()
    unique = dedup_entries(entries)
    circuits = {}
    coverage = {}
    for eid, kind, fields, part_names in unique:
        built = circuits_for_entry(eid, kind, fields)
        for name, netlist in built.items():
            if name in circuits:
                raise SystemExit(f"catalog_sweep: duplicate circuit name {name}")
            circuits[name] = json.dumps(netlist, indent=2) + "\n"
        coverage[eid] = {"kind": kind, "parts": part_names, "circuits": sorted(built)}
    return circuits, coverage


def check_catalog():
    circuits, coverage = build_all()
    drift = []
    expected_dirs = set(circuits)
    existing_dirs = {p.name for p in CATALOG_DIR.iterdir() if p.is_dir()} if CATALOG_DIR.is_dir() else set()
    for stale in sorted(existing_dirs - expected_dirs):
        drift.append(f"stale directory: {stale}")
    for name, text in sorted(circuits.items()):
        path = CATALOG_DIR / name / "netlist.json"
        if not path.is_file() or path.read_text() != text:
            drift.append(f"{name}/netlist.json")
    return {
        "circuits": len(circuits),
        "parameter_sets": len(coverage),
        "excluded": len(EXCLUSIONS),
        "drift": drift,
    }


def write_catalog():
    circuits, coverage = build_all()
    CATALOG_DIR.mkdir(parents=True, exist_ok=True)
    expected_dirs = set(circuits)
    for existing in list(CATALOG_DIR.iterdir()):
        if existing.is_dir() and existing.name not in expected_dirs:
            shutil.rmtree(existing)
    targets = []
    for name, text in sorted(circuits.items()):
        folder = CATALOG_DIR / name
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / "netlist.json"
        if not path.is_file() or path.read_text() != text:
            refs.write_atomic(path, text)
        targets.append(folder)
    report = refs.report_examples(targets, False, refs.ngspice_version())
    report["parameter_sets"] = len(coverage)
    report["excluded"] = len(EXCLUSIONS)
    return report


def main(argv):
    if argv == ["--check"]:
        report = check_catalog()
        print(json.dumps(report, indent=2))
        return 1 if report["drift"] else 0
    if argv:
        sys.exit("usage: catalog_sweep.py [--check]")
    report = write_catalog()
    print(json.dumps(report, indent=2))
    return refs.exit_code(report)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
