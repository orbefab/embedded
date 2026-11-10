"""Self-test cases for packages/engine/scripts/ngspice_refs.py.

The self-test needs ngspice-42 on PATH. The operating-point cases run decks.
The rest do not.
"""

import json
import shutil
import subprocess
import tempfile
from pathlib import Path

from bounce import bounce_vectors


def ngspice_requirement(refs):
    """None when ngspice-42 is on PATH. Otherwise the reason the self-test stops."""
    if shutil.which("ngspice") is None:
        return "ngspice is not on PATH"
    try:
        version = refs.ngspice_version()
    except OSError as exc:
        return f"ngspice --version failed ({exc})"
    except subprocess.CalledProcessError as exc:
        return f"ngspice --version failed with exit {exc.returncode}"
    except SystemExit as exc:
        text = exc.code if isinstance(exc.code, str) else "ngspice --version failed"
        return text
    if version != "ngspice-42":
        return f"ngspice --version is {version}, not ngspice-42"
    return None


def self_test(refs):
    """Skip paths and the operating-point refusal. stdout is one JSON document."""
    reason = ngspice_requirement(refs)
    if reason is not None:
        document = {"ok": False, "cases": [{"name": "ngspice 42", "ok": False, "reason": reason}]}
        print(json.dumps(document, indent=2))
        return 1
    specs = [
        {
            "name": "unknown kind",
            "part": {"name": "t1", "kind": "transformer", "pins": {"a": "a", "b": "gnd"}},
            "commit": [],
            "exit": 0,
            "reason": "kind transformer has no translation",
            "failure": None,
        },
        {
            "name": "firmware kind",
            "part": {"name": "u1", "kind": "atmega328p", "pins": {"PB5": "a", "GND_8": "gnd"}},
            "commit": [],
            "exit": 0,
            "reason": "kind atmega328p runs firmware, which a deck cannot",
            "failure": None,
        },
        {
            "name": "npn blinker",
            "part": {"name": "r1", "kind": "resistor", "pins": {"a": "a", "b": "gnd"}},
            "commit": [],
            "exit": 0,
            "reason": "ngspice aborts this deck at the deck's options with Timestep too small",
            "failure": None,
        },
        {
            "name": "missing pin",
            "part": {"name": "r1", "kind": "resistor", "pins": {"a": "a"}},
            "commit": [],
            "exit": 0,
            "reason": "kind resistor is missing pin b",
            "failure": None,
        },
        {
            "name": "no kind",
            "part": {"name": "r1", "pins": {"a": "a", "b": "gnd"}},
            "commit": [],
            "exit": 0,
            "reason": "part has no kind",
            "failure": None,
        },
        {
            "name": "no pins",
            "part": {"name": "r1", "kind": "resistor"},
            "commit": [],
            "exit": 0,
            "reason": "kind resistor has no pins",
            "failure": None,
        },
        {
            "name": "list kind",
            "part": {"name": "r1", "kind": ["resistor"], "pins": {"a": "a", "b": "gnd"}},
            "commit": [],
            "exit": 0,
            "reason": "netlist.json is not a netlist the engine accepts (TypeError)",
            "failure": None,
        },
        {
            "name": "no name",
            "part": {"kind": "resistor", "pins": {"a": "a", "b": "gnd"}},
            "commit": [],
            "exit": 0,
            "reason": "netlist.json is not a netlist the engine accepts (KeyError)",
            "failure": None,
        },
        {
            "name": "undeclared net",
            "part": {"name": "r1", "kind": "resistor", "pins": {"a": "a", "b": "ghost"}},
            "commit": [],
            "exit": 0,
            "reason": "part r1 pins a net that is not declared",
            "failure": None,
        },
        {
            "name": "committed reference",
            "part": {"name": "t1", "kind": "transformer", "pins": {"a": "a", "b": "gnd"}},
            "commit": ["ngspice.json"],
            "exit": 1,
            "reason": "kind transformer has no translation",
            "failure": "committed ngspice.json was not checked",
        },
        {
            "name": "committed firmware deck",
            "part": {"name": "u1", "kind": "atmega328p", "pins": {"PB5": "a", "GND_8": "gnd"}},
            "commit": ["deck.cir"],
            "exit": 1,
            "reason": "kind atmega328p runs firmware, which a deck cannot",
            "failure": "committed deck.cir was not checked",
        },
        {
            "name": "committed firmware deck and reference",
            "part": {"name": "u1", "kind": "atmega328p", "pins": {"PB5": "a", "GND_8": "gnd"}},
            "commit": ["deck.cir", "ngspice.json"],
            "exit": 1,
            "reason": "kind atmega328p runs firmware, which a deck cannot",
            "failure": "committed deck.cir and ngspice.json were not checked",
        },
        {
            "name": "switch at the operating point",
            "part": {"name": "sw1", "kind": "switch", "pins": {"a": "a", "b": "gnd"}},
            "commit": [],
            "exit": 0,
            "reason": "a switch is translated only for a transient analysis",
            "failure": None,
        },
        {
            "name": "held capacitor at the operating point",
            "part": held_capacitor("a", "gnd"),
            "commit": [],
            "exit": 0,
            "reason": "a held capacitor is translated only for a transient analysis",
            "failure": None,
        },
        {
            "name": "held capacitor off ground",
            "analysis": {"transient": {"stop_ps": 5000, "max_step_ps": 1000, "grid_ps": 1000}},
            "nets": ["gnd", "a", "b"],
            "part": held_capacitor("a", "b"),
            "commit": [],
            "exit": 0,
            "reason": "a held capacitor needs exactly one pin on ground",
            "failure": None,
        },
        {
            "name": "reserved net",
            "analysis": {"transient": {"stop_ps": 5000, "max_step_ps": 1000, "grid_ps": 1000}},
            "nets": ["gnd", "BP1"],
            "part": {"name": "r1", "kind": "resistor", "pins": {"a": "BP1", "b": "gnd"}},
            "commit": [],
            "exit": 0,
            "reason": "a transient deck reserves names starting with ctl_, bp or x_",
            "failure": None,
        },
        {
            "name": "reserved chip prefix",
            "analysis": {"transient": {"stop_ps": 5000, "max_step_ps": 1000, "grid_ps": 1000}},
            "nets": ["gnd", "X_u1_q"],
            "part": {"name": "r1", "kind": "resistor", "pins": {"a": "X_u1_q", "b": "gnd"}},
            "commit": [],
            "exit": 0,
            "reason": "a transient deck reserves names starting with ctl_, bp or x_",
            "failure": None,
        },
        {
            "name": "reserved transistor prefix at the operating point",
            "nets": ["gnd", "a", "x_q1_c"],
            "part": bjt("q1", "npn", "x_q1_c", "a", "gnd"),
            "commit": [],
            "exit": 0,
            "reason": (
                "a deck with a transistor, an inductor, a zener or a linear regulator reserves names starting with x_"
            ),
            "failure": None,
        },
        {
            "name": "chip at the operating point",
            "part": {
                "name": "u1",
                "kind": "ne555",
                "pins": {pin: "a" for pin in refs.PIN_ORDER["ne555"]} | {"gnd": "gnd"},
            },
            "commit": [],
            "exit": 0,
            "reason": "a chip is translated only for a transient analysis",
            "failure": None,
        },
        {
            "name": "pulse period inside its width",
            "analysis": {"transient": {"stop_ps": 10000, "max_step_ps": 1000, "grid_ps": 1000}},
            "part": {
                "name": "v1",
                "kind": "voltage_source",
                "pins": {"p": "a", "n": "gnd"},
                "voltage": {
                    "kind": "pulse",
                    "initial": {"value": 0.0},
                    "pulsed": {"value": 1.0},
                    "delay_ps": 1000,
                    "width_ps": 2000,
                    "period_ps": 0,
                },
            },
            "commit": [],
            "exit": 0,
            "reason": "a pulse period does not exceed its width",
            "failure": None,
        },
        {
            "name": "pulse denser than the deck cap",
            "analysis": {"transient": {"stop_ps": 1000000, "max_step_ps": 1000, "grid_ps": 1000}},
            "part": {
                "name": "v1",
                "kind": "voltage_source",
                "pins": {"p": "a", "n": "gnd"},
                "voltage": {
                    "kind": "pulse",
                    "initial": {"value": 0.0},
                    "pulsed": {"value": 1.0},
                    "delay_ps": 1,
                    "width_ps": 0,
                    "period_ps": 1,
                },
            },
            "commit": [],
            "exit": 0,
            "reason": "a waveform has more than 100000 knots before the stop",
            "failure": None,
        },
    ]
    cases = []
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for spec in specs:
            example = root / spec["name"].replace(" ", "-")
            example.mkdir()
            netlist = {
                "analysis": spec.get("analysis", {"op": {}}),
                "ground": "gnd",
                "nets": spec.get("nets", ["gnd", "a"]),
                "parts": [spec["part"]],
            }
            (example / "netlist.json").write_text(json.dumps(netlist))
            for name in spec["commit"]:
                (example / name).write_text("{}\n")
            report = refs.report_examples([example], True, "ngspice-42")
            code = refs.exit_code(report)
            skipped = report["skipped"]
            got_reason = skipped[0]["reason"] if len(skipped) == 1 else None
            got_failure = skipped[0].get("failure") if len(skipped) == 1 else None
            ok = (
                code == spec["exit"]
                and got_reason == spec["reason"]
                and got_failure == spec["failure"]
                and report["checked"] == []
                and report["drift"] == []
            )
            cases.append(
                {
                    "name": spec["name"],
                    "ok": ok,
                    "exit": code,
                    "reason": got_reason,
                    "failure": got_failure,
                }
            )
    cases.append(initial_conditions(refs))
    cases.append(source_decks(refs))
    cases.append(serial_boundaries(refs))
    cases.append(source_jump_times(refs))
    cases.append(potentiometer_deck(refs))
    cases.append(diode_deck(refs))
    cases.append(zener_deck(refs))
    cases.append(regulator_deck(refs))
    cases.append(transistor_deck(refs))
    cases.append(nmos_deck(refs))
    cases.append(inductor_deck(refs))
    cases.append(corner_check(refs))
    cases.append(relay_deck(refs))
    cases.append(contact_edges(refs))
    cases.append(bare_flyback(refs))
    cases.append(bounce_vectors())
    cases.append(vcd_reader(refs))
    cases.append(engine_grid(refs))
    cases.extend(operating_point_fallback(refs))
    document = {"ok": all(case["ok"] for case in cases), "cases": cases}
    print(json.dumps(document, indent=2))
    return 0 if document["ok"] else 1


def engine_grid(refs):
    """The engine's rows keep what the comparison scores.

    The row on a jump reads ngspice's point there, the level before the ramp.
    A spike between rows still sets the stored span, `p` included. A held
    capacitor's span skips t = 0. 2745 points stays dense, because that is the
    nmos-pulse fixture. A chip stays on the deck's own linearize. A grid that
    does not divide the stop is refused.
    """
    netlist = {
        "analysis": {"transient": {"stop_ps": 2_000_000_000, "grid_ps": 1_000_000_000}},
        "parts": [
            {"name": "r1", "kind": "resistor", "pins": {"a": "a", "b": "gnd"}},
            {"name": "c1", "kind": "capacitor", "pins": {"a": "a", "b": "gnd"}, "initial": {"value": 0.0}},
        ],
    }
    times = [0.0, 1e-3, 1e-3 + 1e-9, 1.5e-3, 2e-3]
    volts = [0.0, 0.0, 1.0, 3.0, 1.0]
    values = {
        "#net.a": volts,
        "#net.gnd": [0.0] * 5,
        "#r1.v": volts,
        "#r1.i": [v / 1000 for v in volts],
        "#c1.v": volts,
        "#c1.i": [5.0, 0.0, 2.0, 0.0, 0.0],
    }
    spans = refs.dense_spans(netlist, values, {})
    t_s, rows, _ = refs.on_engine_grid(netlist, times, values, {})
    refused = False
    try:
        refs.on_engine_grid({"analysis": {"transient": {"stop_ps": 3, "grid_ps": 2}}, "parts": []}, times, {}, {})
    except SystemExit:
        refused = True
    ok = (
        refs.POINT_CAP == 3000
        and not refs.store_on_engine_grid(2745, False)
        and not refs.store_on_engine_grid(3000, False)
        and refs.store_on_engine_grid(3001, False)
        and not refs.store_on_engine_grid(10000, True)
        and t_s == [0.0, 1e-3, 2e-3]
        and rows["#r1.v"] == [0.0, 0.0, 1.0]
        and spans["#r1.v"] == 3.0
        and spans["#r1.p"] == 3.0 * (3.0 / 1000)
        and spans["#c1.i"] == 2.0
        and set(spans) == set(values) | {"#r1.p", "#c1.p"}
        and refused
    )
    return {"name": "engine grid", "ok": ok, "time": t_s, "v": rows["#r1.v"], "span": spans}


def quantity(value):
    return {"value": value}


def npn_led_netlist(voltage, transient=False):
    """`packages/engine/tests/circuits/npn-led` with the pin source at `voltage`. The part values are that netlist's.

    A transient run stops at 1 us. Its operating point is the same circuit.
    """
    led = {
        "name": "led1",
        "kind": "led",
        "pins": {"anode": "vled", "cathode": "c"},
        "saturation_current": quantity(6.711e-10),
        "emission_coefficient": quantity(4.427),
        "series_resistance": quantity(2.115),
    }
    q1 = {
        "name": "q1",
        "kind": "npn",
        "pins": {"collector": "c", "base": "b", "emitter": "gnd"},
        "saturation_current": quantity(8.095e-15),
        "forward_beta": quantity(173.2),
        "reverse_beta": quantity(0.2704),
        "forward_emission": quantity(1.0),
        "reverse_emission": quantity(1.0),
        "early_voltage": quantity(100.0),
        "base_resistance": quantity(5.707),
        "collector_resistance": quantity(1.531),
        "emitter_resistance": quantity(0.867),
        "base_emitter_capacitance": quantity(8e-12),
        "base_collector_capacitance": quantity(4e-12),
    }
    analysis = {"op": {}}
    if transient:
        analysis = {"transient": {"stop_ps": 1_000_000, "max_step_ps": 1000, "grid_ps": 1000}}
    return {
        "analysis": analysis,
        "ground": "gnd",
        "nets": ["gnd", "b", "c", "pin", "vcc", "vled"],
        "parts": [
            {
                "name": "v1",
                "kind": "voltage_source",
                "pins": {"p": "vcc", "n": "gnd"},
                "voltage": quantity(5.0),
            },
            {
                "name": "vpin",
                "kind": "voltage_source",
                "pins": {"p": "pin", "n": "gnd"},
                "voltage": quantity(voltage),
            },
            {
                "name": "rb",
                "kind": "resistor",
                "pins": {"a": "pin", "b": "b"},
                "resistance": quantity(1000.0),
            },
            {
                "name": "r1",
                "kind": "resistor",
                "pins": {"a": "vcc", "b": "vled"},
                "resistance": quantity(220.0),
            },
            led,
            q1,
        ],
    }


def divider_netlist():
    return {
        "analysis": {"op": {}},
        "ground": "gnd",
        "nets": ["gnd", "mid"],
        "parts": [
            {
                "name": "v1",
                "kind": "voltage_source",
                "pins": {"p": "mid", "n": "gnd"},
                "voltage": quantity(5.0),
            },
            {
                "name": "r1",
                "kind": "resistor",
                "pins": {"a": "mid", "b": "gnd"},
                "resistance": quantity(1000.0),
            },
        ],
    }


def exit_message(refs, example, check):
    try:
        refs.report_examples([example], check, "ngspice-42")
    except SystemExit as exc:
        return exc.code if isinstance(exc.code, str) else None
    return None


def write_netlist(folder, netlist):
    folder.mkdir()
    (folder / "netlist.json").write_text(json.dumps(netlist))


def refusal_matches(refs, folder, lines):
    """Both modes raise the same message, and the folder keeps only netlist.json."""
    generation = exit_message(refs, folder, False)
    check = exit_message(refs, folder, True)
    written = sorted(path.name for path in folder.iterdir())
    expected = f"{folder.resolve().as_posix()}: ngspice operating point did not converge: {'; '.join(lines)}"
    ok = generation == expected and check == expected and written == ["netlist.json"]
    return ok, generation, check, written


def shown(name, ok, detail):
    case = {"name": name, "ok": ok}
    if not ok:
        case.update(detail)
    return case


def operating_point_fallback(refs):
    """A pin at 0 V on the npn-led circuit is refused. A plain divider is not.

    A start line paired with its completed line is still a refusal, and the
    start line comes back. The same circuit as a transient is refused. A pin
    at 1e-12 V that completes dynamic gmin stepping is refused.
    """
    pin_lines = (
        "Note: Starting dynamic gmin stepping",
        "Warning: Dynamic gmin stepping failed",
        "Note: Starting true gmin stepping",
        "Warning: True gmin stepping failed",
        "Note: Starting source stepping",
        "Warning: source stepping failed",
        "Note: Transient op started",
    )
    pairs = (
        ("Note: Starting dynamic gmin stepping", "Note: Dynamic gmin stepping completed"),
        ("Note: Starting true gmin stepping", "Note: True gmin stepping completed"),
        ("Note: Starting spice3 gmin stepping", "Note: spice3 gmin stepping completed"),
        ("Note: Starting source stepping", "Note: Source stepping completed"),
        ("Note: Transient op started", "Note: Transient op finished successfully"),
    )
    gmin_line = ("Note: Starting dynamic gmin stepping",)
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        pin = root / "npn-led-pin-0"
        write_netlist(pin, npn_led_netlist(0.0))
        plain = root / "divider"
        write_netlist(plain, divider_netlist())
        pin_ok, generation, check, written = refusal_matches(refs, pin, pin_lines)
        plain_files = refs.generate(plain, "ngspice-42")
        recovered = root / "npn-led-pin-1e-12"
        write_netlist(recovered, npn_led_netlist(1e-12))
        recovered_ok, recovered_generation, recovered_check, recovered_written = refusal_matches(
            refs, recovered, gmin_line
        )
        transient = root / "npn-led-pin-0-tran"
        write_netlist(transient, npn_led_netlist(0.0, transient=True))
        transient_ok, transient_generation, transient_check, transient_written = refusal_matches(
            refs, transient, pin_lines
        )
    paired = refs.op_fallback_lines("\n".join(line for pair in pairs for line in pair))
    other_lines = [
        "Note: Starting spice3 gmin stepping",
        "Warning: spice3 gmin stepping failed",
        "Error: Transient op failed, timestep too small",
        "Error: The operating point could not be simulated successfully.",
    ]
    other = refs.op_fallback_lines("\n".join(other_lines))
    starts = [start for start, _completed in pairs]
    plain_ok = "op" in json.loads(plain_files["ngspice.json"])
    paired_ok = paired == starts
    other_ok = other == other_lines
    return [
        shown(
            "operating point fallback",
            pin_ok and plain_ok and paired_ok and other_ok,
            {
                "generation": generation,
                "check": check,
                "written": written,
                "paired": paired,
                "other": other,
                "plain": plain_ok,
            },
        ),
        shown(
            "completed gmin stepping",
            recovered_ok,
            {
                "generation": recovered_generation,
                "check": recovered_check,
                "written": recovered_written,
            },
        ),
        shown(
            "transient operating point fallback",
            transient_ok,
            {
                "generation": transient_generation,
                "check": transient_check,
                "written": transient_written,
            },
        ),
    ]


def held_capacitor(a, b):
    return {
        "name": "c1",
        "kind": "capacitor",
        "pins": {"a": a, "b": b},
        "capacitance": {"value": 1e-6},
        "initial": {"value": 1.5},
    }


def initial_conditions(refs):
    """A held capacitor's `.ic` line, with either pin on ground."""
    lines = []
    for a, b in (("a", "gnd"), ("gnd", "a")):
        netlist = {
            "analysis": {"transient": {"stop_ps": 5000, "max_step_ps": 1000, "grid_ps": 1000}},
            "ground": "gnd",
            "nets": ["gnd", "a"],
            "parts": [held_capacitor(a, b)],
        }
        deck, _ = refs.translate("held", netlist)
        lines += [line for line in deck.splitlines() if line.startswith(".ic")]
    return {"name": "initial conditions", "ok": lines == [".ic v(a)=1.5", ".ic v(a)=-1.5"]}


def source_decks(refs):
    """Each source spelling's deck line, corners and saves, at the operating point and in a
    transient, and the current source's negated current."""
    step = {"kind": "step", "initial": {"value": 0.0}, "final": {"value": 5.0}, "at_ps": 2000}
    pulse = {
        "kind": "pulse",
        "initial": {"value": 0.5},
        "pulsed": {"value": 1.0},
        "delay_ps": 2000,
        "width_ps": 2000,
        "period_ps": 5000,
    }
    op = {"op": {}}
    tran = {"transient": {"stop_ps": 9000, "max_step_ps": 1000, "grid_ps": 1000}}

    def deck(analysis, kind, key, value):
        part = {"name": "s1", "kind": kind, "pins": {"p": "a", "n": "gnd"}, key: value}
        netlist = {"analysis": analysis, "ground": "gnd", "nets": ["gnd", "a"], "parts": [part]}
        text, mapping = refs.translate("sources", netlist)
        kept = ("vs1", "is1", "vbp", "save")
        return [line for line in text.splitlines() if line.startswith(kept)], mapping

    corners = [
        f"vbp{k} bp{k} 0 pwl({at}p 0 {at + 1}p 0)" for k, at in enumerate((2000, 3000, 4000, 5000, 7000, 8000, 9000))
    ]
    cases = [
        (deck(op, "voltage_source", "voltage", {"value": 5.0}), ["vs1 a 0 dc 5.0", "save all"]),
        (
            deck(op, "current_source", "current", {"value": 0.001}),
            ["is1 0 a dc 0.001", "save all @is1[current]"],
        ),
        (deck(op, "voltage_source", "voltage", step), ["vs1 a 0 dc 0.0", "save all"]),
        (
            deck(
                op,
                "voltage_source",
                "voltage",
                {"kind": "steps", "initial": {"value": 1.5}, "events": []},
            ),
            ["vs1 a 0 dc 1.5", "save all"],
        ),
        (
            deck(tran, "voltage_source", "voltage", step),
            [
                "vs1 a 0 dc 0.0 pwl(0p 0.0 2000p 0.0 3000p 5.0)",
                "vbp0 bp0 0 pwl(2000p 0 2001p 0)",
                "vbp1 bp1 0 pwl(3000p 0 3001p 0)",
                "save v(a) i(vs1)",
            ],
        ),
        (
            deck(
                tran,
                "voltage_source",
                "voltage",
                {
                    "kind": "steps",
                    "initial": {"value": 0.0},
                    "events": [
                        {"t_ps": 2000, "level": {"value": 5.0}},
                        {"t_ps": 5000, "level": {"value": 1.0}},
                    ],
                },
            ),
            [
                "vs1 a 0 dc 0.0 pwl(0p 0.0 2000p 0.0 3000p 5.0 5000p 5.0 6000p 1.0)",
                "vbp0 bp0 0 pwl(2000p 0 2001p 0)",
                "vbp1 bp1 0 pwl(3000p 0 3001p 0)",
                "vbp2 bp2 0 pwl(5000p 0 5001p 0)",
                "vbp3 bp3 0 pwl(6000p 0 6001p 0)",
                "save v(a) i(vs1)",
            ],
        ),
        (
            deck(
                {"transient": {"stop_ps": 30000, "max_step_ps": 1000, "grid_ps": 1000}},
                "voltage_source",
                "voltage",
                {
                    "kind": "serial",
                    "mark": {"value": 5.0},
                    "space": {"value": 0.0},
                    "events": [{"t_ps": 2000, "baud": 500000000, "frame": "8N1", "bytes": [0]}],
                },
            ),
            [
                "vs1 a 0 dc 5.0 pwl(0p 5.0 2000p 5.0 3000p 0.0 20000p 0.0 21000p 5.0)",
                "vbp0 bp0 0 pwl(2000p 0 2001p 0)",
                "vbp1 bp1 0 pwl(3000p 0 3001p 0)",
                "vbp2 bp2 0 pwl(20000p 0 20001p 0)",
                "vbp3 bp3 0 pwl(21000p 0 21001p 0)",
                "save v(a) i(vs1)",
            ],
        ),
        (
            deck(
                op,
                "voltage_source",
                "voltage",
                {
                    "kind": "serial",
                    "mark": {"value": 5.0},
                    "space": {"value": 0.0},
                    "events": [],
                },
            ),
            ["vs1 a 0 dc 5.0", "save all"],
        ),
        (
            deck(tran, "current_source", "current", pulse),
            [
                ("is1 0 a dc 0.5 pwl(0p 0.5 2000p 0.5 3000p 1.0 4000p 1.0 5000p 0.5 7000p 0.5 8000p 1.0 9000p 1.0)"),
                *corners,
                "save v(a) @is1[current]",
            ],
        ),
    ]
    ok = all(lines == want for (lines, _), want in cases)
    ok = ok and cases[1][0][1]["i(@is1[current])"] == refs.Negated("#s1.i")
    return {"name": "source decks", "ok": ok}


def serial_boundaries(refs):
    """Serial jump times and levels, computed here and not by the engine."""
    byte = {
        "kind": "serial",
        "mark": {"value": 5.0},
        "space": {"value": 0.0},
        "events": [{"t_ps": 1000000000, "baud": 9600, "frame": "8N1", "bytes": [0x31]}],
    }
    jumps = list(refs.jumps(byte))
    want = [
        (1000000000, 5.0, 0.0),
        (1104166666, 0.0, 5.0),
        (1208333333, 5.0, 0.0),
        (1520833333, 0.0, 5.0),
        (1729166666, 5.0, 0.0),
        (1937500000, 0.0, 5.0),
    ]
    ok = jumps == want

    def counted(data):
        wave = {
            "kind": "serial",
            "mark": {"value": 5.0},
            "space": {"value": 0.0},
            "events": [{"t_ps": 10000, "baud": 500000000, "frame": "8N1", "bytes": data}],
        }
        return len(list(refs.jumps(wave)))

    ok = ok and counted([0x00]) == 2 and counted([0xFF]) == 2 and counted([0x55]) == 10
    text = {
        "kind": "serial",
        "mark": {"value": 5.0},
        "space": {"value": 0.0},
        "events": [{"t_ps": 5000000000, "baud": 9600, "frame": "8N1", "bytes": [0x30, 0x68, 0x69]}],
    }
    times = [at for at, _, _ in refs.jumps(text)]
    ok = ok and 7083333333 in times
    try:
        list(refs.jumps({**byte, "events": [{**byte["events"][0], "frame": "8E1"}]}))
    except refs.Skip:
        pass
    else:
        ok = False
    return {"name": "serial boundaries", "ok": ok}


def source_jump_times(refs):
    """`source_jumps` stops at the stop and keeps the floor times."""
    serial = {
        "kind": "serial",
        "mark": {"value": 5.0},
        "space": {"value": 0.0},
        "events": [{"t_ps": 1000000000, "baud": 9600, "frame": "8N1", "bytes": [0x31]}],
    }
    netlist = {
        "analysis": {"transient": {"stop_ps": 1_600_000_000}},
        "parts": [{"voltage": serial}],
    }
    pulse = {
        "kind": "pulse",
        "initial": {"value": 0.0},
        "pulsed": {"value": 1.0},
        "delay_ps": 1000,
        "width_ps": 2000,
        "period_ps": 5000,
    }
    pulsed = {
        "analysis": {"transient": {"stop_ps": 12000}},
        "parts": [{"voltage": pulse}, {"current": serial}],
    }
    ok = refs.source_jumps(netlist) == [
        1000000000,
        1104166666,
        1208333333,
        1520833333,
    ] and refs.source_jumps(pulsed) == [1000, 3000, 6000, 8000, 11000]
    return {"name": "source jump times", "ok": ok}


def potentiometer_deck(refs):
    """One potentiometer is two resistors. A resistor with one of those names is skipped."""
    part = {
        "name": "rv1",
        "kind": "potentiometer",
        "pins": {"a": "vcc", "wiper": "mid", "b": "gnd"},
        "resistance": {"value": 10000.0},
        "position": {"value": 0.3},
    }
    netlist = {
        "analysis": {"op": {}},
        "ground": "gnd",
        "nets": ["gnd", "vcc", "mid"],
        "parts": [part],
    }
    text, _mapping = refs.translate("pot", netlist)
    lines = [line for line in text.splitlines() if line.startswith("rrv1")]
    ok = lines == ["rrv1_a vcc mid 3000.0", "rrv1_b mid 0 7000.0"]
    ok = ok and not any("@rrv1_" in line for line in text.splitlines())
    clash = {
        **netlist,
        "parts": [
            part,
            {
                "name": "rv1_a",
                "kind": "resistor",
                "pins": {"a": "vcc", "b": "gnd"},
                "resistance": {"value": 1000.0},
            },
        ],
    }
    try:
        refs.translate("pot", clash)
    except refs.Skip as skip:
        ok = ok and str(skip) == "two device names are equal ignoring case"
    else:
        ok = False
    missing = {
        **netlist,
        "parts": [{**part, "pins": {"a": "vcc", "b": "gnd"}}],
    }
    try:
        refs.translate("pot", missing)
    except refs.Skip as skip:
        ok = ok and str(skip) == "kind potentiometer is missing pin wiper"
    else:
        ok = False
    fixed = {
        **part,
        "position": {"kind": "steps", "initial": {"value": 0.3}, "events": []},
    }
    text, _mapping = refs.translate("pot", {**netlist, "parts": [fixed]})
    lines = [line for line in text.splitlines() if line.startswith("rrv1")]
    ok = ok and lines == ["rrv1_a vcc mid 3000.0", "rrv1_b mid 0 7000.0"]
    moving = {
        **part,
        "position": {
            "kind": "steps",
            "initial": {"value": 0.25},
            "events": [{"t_ps": 2000, "level": {"value": 0.75}}],
        },
    }
    try:
        refs.translate("pot", {**netlist, "parts": [moving]})
    except refs.Skip as skip:
        ok = ok and str(skip) == "a moving potentiometer has one position in a deck"
    else:
        ok = False
    return {"name": "potentiometer deck", "ok": ok}


def bjt(name, kind, c, b, e):
    part = {"name": name, "kind": kind, "pins": {"collector": c, "base": b, "emitter": e}}
    for k, (_, field) in enumerate(refs_card()):
        part[field] = {"value": float(k + 1)}
    return part


def refs_card():
    from ngspice_refs import BJT_CARD

    return BJT_CARD


def diode_deck(refs):
    """A diode's device line, its model card, and the shunt on every external net, in an
    operating point deck and in a transient one."""
    netlist = {
        "analysis": {"op": {}},
        "ground": "gnd",
        "nets": ["gnd", "a", "c"],
        "parts": [
            {
                "name": "D1",
                "kind": "diode",
                "pins": {"anode": "a", "cathode": "c"},
                "saturation_current": {"value": 1e-14},
                "emission_coefficient": {"value": 1.0},
                "series_resistance": {"value": 1.0},
            }
        ],
    }
    text, mapping = refs.translate("diode", netlist)
    want = [
        "dd1 a c dmod_d1",
        ".model dmod_d1 d (is=1e-14 n=1.0 rs=1.0)",
        "* The engine's gmin on every node, beside the gmin ngspice puts across each junction.",
        "rx_gmin_a a 0 1000000000000.0",
        "rx_gmin_c c 0 1000000000000.0",
    ]
    lines = text.splitlines()
    ok = lines[2 : 2 + len(want)] == want
    ok = ok and mapping["i(@dd1[id])"] == "#D1.i"
    transient = dict(netlist, analysis={"transient": {"stop_ps": 1_000_000, "max_step_ps": 1000, "grid_ps": 1000}})
    text, _ = refs.translate("diode", transient)
    ok = ok and all(line in text.splitlines() for line in want[2:])
    return {"name": "diode deck", "ok": ok}


def zener_deck(refs):
    """Two diodes and two resistors, shunts on the external nets only, and #.i on the series resistor."""
    import math

    part = {
        "name": "DZ1",
        "kind": "zener",
        "pins": {"anode": "a", "cathode": "c"},
        "saturation_current": {"value": 1e-14},
        "emission_coefficient": {"value": 1.0},
        "series_resistance": {"value": 1.0},
        "zener_voltage": {"value": 5.0},
        "zener_current": {"value": 0.05},
        "zener_impedance": {"value": 10.0},
        "breakdown_resistance": {"value": 4.0},
    }
    netlist = {
        "analysis": {"op": {}},
        "ground": "gnd",
        "nets": ["gnd", "a", "c"],
        "parts": [part],
    }
    text, mapping = refs.translate("zener", netlist)
    rs = 1.0
    rb = 4.0
    vz = 5.0
    izt = 0.05
    zzt = 10.0
    nbvt = (zzt - rs - rb) * izt
    isb = izt / math.expm1((vz - izt * (rs + rb)) / nbvt)
    nbv = nbvt / refs.VT
    want = [
        "rx_dz1_s a x_dz1 1.0",
        "dx_dz1_f x_dz1 c dmodf_dz1",
        "rx_dz1_b x_dz1_b c 4.0",
        "dx_dz1_b x_dz1_b x_dz1 dmodb_dz1",
        ".model dmodf_dz1 d (is=1e-14 n=1.0)",
        f".model dmodb_dz1 d (is={isb!r} n={nbv!r})",
        "* The engine's gmin on every node, beside the gmin ngspice puts across each junction.",
        "rx_gmin_a a 0 1000000000000.0",
        "rx_gmin_c c 0 1000000000000.0",
    ]
    lines = text.splitlines()
    ok = lines[2 : 2 + len(want)] == want
    ok = ok and "rx_gmin_x_dz1" not in text
    ok = ok and mapping["i(@rx_dz1_s[i])"] == "#DZ1.i"
    ok = ok and mapping["v(x_dz1)"] is None and mapping["v(x_dz1_b)"] is None
    reserved = dict(netlist, nets=["gnd", "a", "x_dz1"])
    reserved["parts"] = [dict(part, pins={"anode": "a", "cathode": "x_dz1"})]
    try:
        refs.translate("zener", reserved)
    except refs.Skip as skip:
        ok = ok and str(skip) == (
            "a deck with a transistor, an inductor, a zener or a linear regulator reserves names starting with x_"
        )
    else:
        ok = False
    return {"name": "zener deck", "ok": ok}


def regulator_deck(refs):
    """The branch form, the external shunts, and a real operating point at 1 A and at 1 pA."""
    part = {
        "name": "u1",
        "kind": "linear_regulator",
        "pins": {"in": "vin", "out": "vout", "gnd": "gnd"},
        "output_voltage": {"value": 3.3},
        "line_regulation": {"value": 0.001 / 10.25},
        "output_resistance": {"value": 0.00125},
        "dropout_voltage": {"value": 1.1 - 0.1 / 7},
        "dropout_resistance": {"value": 0.1 / 0.7},
        "current_limit": {"value": 1.2},
        "quiescent_current": {"value": 0.005},
        "ground_current_ratio": {"value": 0.0},
    }

    def netlist(load):
        return {
            "analysis": {"op": {}},
            "ground": "gnd",
            "nets": ["gnd", "vin", "vout"],
            "parts": [
                {"name": "v1", "kind": "voltage_source", "pins": {"p": "vin", "n": "gnd"}, "voltage": {"value": 5.0}},
                {"name": "rl", "kind": "resistor", "pins": {"a": "vout", "b": "gnd"}, "resistance": {"value": load}},
                part,
            ],
        }

    text, mapping = refs.translate("regulator", netlist(1.0))
    lines = text.splitlines()
    ok = text.count(".func sp(") == 1
    ok = ok and any(line.startswith("bx_u1_v ") and "smin(" in line and "i(vx_u1)" in line for line in lines)
    ok = ok and "vx_u1 x_u1 vout dc 0" in lines
    ok = ok and "fx_u1 vin 0 vx_u1 1" in lines
    ok = ok and any(line.startswith("bx_u1_g ") for line in lines)
    ok = ok and "rx_gmin_x_u1" not in text and "rx_gmin_x_u1_g" not in text
    ok = ok and mapping["i(vx_u1)"] == refs.RegulatorPass("#u1")
    ok = ok and mapping["i(vx_u1_g)"] == refs.RegulatorGround("#u1")
    ok = ok and mapping["v(x_u1)"] is None and mapping["v(x_u1_g)"] is None
    reserved = netlist(1.0)
    reserved["nets"] = ["gnd", "vin", "x_u1"]
    reserved["parts"][2] = dict(part, pins={"in": "vin", "out": "x_u1", "gnd": "gnd"})
    try:
        refs.translate("regulator", reserved)
    except refs.Skip as skip:
        ok = ok and "linear regulator" in str(skip)
    else:
        ok = False

    def ran(load):
        try:
            (vectors,) = refs.run_ngspice(refs.translate("regulator", netlist(load))[0], ["op.raw"], "regulator")
        except SystemExit:
            return None
        return vectors

    limited = ran(1.0)
    opened = ran(1e9)
    ok = ok and limited is not None and opened is not None
    if limited and opened:
        ok = ok and 1.15 < limited["v(vout)"][0] < 1.25
        ok = ok and abs(opened["v(vout)"][0] - 3.300166) < 1e-4
        ok = ok and abs(opened["i(vx_u1_g)"][0] - 0.005) < 1e-6
    soft = "\n".join(
        [
            ".options temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12",
            ".func sp(x) {ln(1+exp(-abs(x))) + (x>0 ? x : 0)}",
            "bhi hi 0 v = sp(1e12)",
            "blo lo 0 v = sp(-1e12)",
            "rhi hi 0 1e12",
            "rlo lo 0 1e12",
            ".control",
            "save all",
            "op",
            "set filetype=ascii",
            "write op.raw",
            ".endc",
            ".end",
            "",
        ]
    )
    try:
        (vectors,) = refs.run_ngspice(soft, ["op.raw"], "softplus")
    except SystemExit:
        ok = False
    else:
        ok = ok and abs(vectors["v(hi)"][0] - 1e12) / 1e12 < 1e-6
        ok = ok and abs(vectors["v(lo)"][0]) < 1e-6
    return {"name": "regulator deck", "ok": ok}


def transistor_deck(refs):
    """Each BJT's ammeters, device line and model card, the shunt on every node, and the refs
    its vectors map to. The ammeter nodes' voltages map to nothing."""
    netlist = {
        "analysis": {"op": {}},
        "ground": "gnd",
        "nets": ["gnd", "C", "b", "e"],
        "parts": [bjt("Q1", "npn", "C", "b", "gnd"), bjt("q2", "pnp", "gnd", "b", "e")],
    }
    text, mapping = refs.translate("bjt", netlist)
    card = "is=1.0 bf=2.0 br=3.0 nf=4.0 nr=5.0 vaf=6.0 rb=7.0 rc=8.0 re=9.0 cje=10.0 cjc=11.0 mje=0 mjc=0"
    want = [
        "vx_q1_c c x_q1_c dc 0",
        "vx_q1_b b x_q1_b dc 0",
        "qq1 x_q1_c x_q1_b 0 qmod_q1",
        f".model qmod_q1 npn ({card})",
        "vx_q2_c 0 x_q2_c dc 0",
        "vx_q2_b b x_q2_b dc 0",
        "qq2 x_q2_c x_q2_b e qmod_q2",
        f".model qmod_q2 pnp ({card})",
        "* The engine's gmin on every node, beside the gmin ngspice puts across each junction.",
        "rx_gmin_c c 0 1000000000000.0",
        "rx_gmin_b b 0 1000000000000.0",
        "rx_gmin_e e 0 1000000000000.0",
        ".control",
        "save all i(vx_q1_c) i(vx_q1_b) i(vx_q2_c) i(vx_q2_b)",
    ]
    ok = text.splitlines()[2 : 2 + len(want)] == want
    ok = ok and mapping["i(vx_q1_c)"] == "#Q1.ic" and mapping["i(vx_q1_b)"] == "#Q1.ib"
    ok = ok and mapping["i(vx_q2_c)"] == "#q2.ic" and mapping["v(x_q2_b)"] is None
    values = {"#net.C": [5.0], "#net.b": [0.7], "#net.e": [2.0], "#Q1.ic": [0.01], "#Q1.ib": [1e-4]}
    values |= {"#q2.ic": [-0.02], "#q2.ib": [-2e-4]}
    op = refs.op_point(netlist, values)
    ok = ok and op["#Q1.p"] == 5.0 * 0.01 + 0.7 * 1e-4
    ok = ok and op["#q2.p"] == (0.0 - 2.0) * -0.02 + (0.7 - 2.0) * -2e-4
    ok = ok and not any(key.endswith(".v") for key in op)
    return {"name": "transistor deck", "ok": ok}


def nmos_part(name, d, g, s):
    part = {"name": name, "kind": "nmos", "pins": {"drain": d, "gate": g, "source": s}}
    for field, value in (
        ("threshold_voltage", 1.08),
        ("transconductance_parameter", 0.0468),
        ("channel_length_modulation", 0.0),
        ("gate_source_capacitance", 55e-12),
        ("gate_drain_capacitance", 5e-12),
        ("drain_source_capacitance", 20e-12),
    ):
        part[field] = {"value": value}
    return part


def nmos_deck(refs):
    """An nmos ammeter pair, level-1 card, three capacitors on the device side, and drain power."""
    netlist = {
        "analysis": {"op": {}},
        "ground": "gnd",
        "nets": ["gnd", "d", "g"],
        "parts": [nmos_part("m1", "d", "g", "gnd")],
    }
    text, mapping = refs.translate("nmos", netlist)
    want = [
        "vx_m1_d d x_m1_d dc 0",
        "vx_m1_g g x_m1_g dc 0",
        "mm1 x_m1_d x_m1_g 0 0 mmod_m1 l=100u w=100u",
        ".model mmod_m1 nmos (level=1 vto=1.08 kp=0.0468 lambda=0.0 is=0)",
        "cm1_gs x_m1_g 0 5.5e-11",
        "cm1_gd x_m1_g x_m1_d 5e-12",
        "cm1_ds x_m1_d 0 2e-11",
        "* The engine's gmin on every node, beside the gmin ngspice puts across each junction.",
        "rx_gmin_d d 0 1000000000000.0",
        "rx_gmin_g g 0 1000000000000.0",
        ".control",
        "save all i(vx_m1_d) i(vx_m1_g)",
    ]
    ok = text.splitlines()[2 : 2 + len(want)] == want
    ok = ok and mapping["i(vx_m1_d)"] == "#m1.id"
    ok = ok and mapping["i(vx_m1_g)"] == refs.GateCurrent("#m1")
    ok = ok and mapping["v(x_m1_d)"] is None and mapping["v(x_m1_g)"] is None
    values = {"#net.d": [5.0], "#net.g": [2.0], "#m1.id": [0.01]}
    op = refs.op_point(netlist, values)
    ok = ok and op["#m1.p"] == 5.0 * 0.01
    ok = ok and not any(key.endswith(".v") for key in op)
    ok = ok and "#m1.ig" not in op
    return {"name": "nmos deck", "ok": ok}


def rl_netlist(analysis):
    step = {"kind": "step", "initial": {"value": 0.0}, "final": {"value": 5.0}, "at_ps": 2000}
    return {
        "analysis": analysis,
        "ground": "gnd",
        "nets": ["gnd", "a"],
        "parts": [
            {"name": "v1", "kind": "voltage_source", "pins": {"p": "a", "n": "gnd"}, "voltage": step},
            {
                "name": "l1",
                "kind": "inductor",
                "pins": {"a": "a", "b": "gnd"},
                "inductance": {"value": 0.5},
                "series_resistance": {"value": 70.0},
            },
        ],
    }


def inductor_deck(refs):
    """The series resistance and the inductance, and a transient deck's reltol, its breakpoint
    source per corner held to the stop, and its step cap. An operating point keeps the plain
    options."""
    tran = {"transient": {"stop_ps": 60000000, "max_step_ps": 1000000, "grid_ps": 1000000}}
    text, mapping = refs.translate("rl", rl_netlist(tran))
    want = [
        "* rl: generated by packages/engine/scripts/ngspice_refs.py from netlist.json. Do not edit.",
        f".options {refs.INDUCTOR_OPTIONS}",
        "rx_l1_s a x_l1 70.0",
        "ll1 x_l1 0 0.5",
        "vv1 a 0 dc 0.0 pwl(0p 0.0 2000p 0.0 3000p 5.0)",
        (
            "* One source per corner. Its first point is the corner, "
            "a breakpoint from t = 0, and its second point is the stop."
        ),
        "vbp0 bp0 0 pwl(2000p 0 60000000p 0)",
        "vbp1 bp1 0 pwl(3000p 0 60000000p 0)",
        ".control",
        "save v(a) i(vv1) i(ll1)",
        "tran 60000p 60000000p 0 1000p",
    ]
    ok = text.splitlines()[: len(want)] == want
    ok = ok and mapping["i(ll1)"] == "#l1.i" and mapping["v(x_l1)"] is None
    ok = ok and refs.INDUCTOR_OPTIONS == refs.OPTIONS.replace("reltol=1e-9", "reltol=1e-5")
    text, _ = refs.translate("rl", rl_netlist({"op": {}}))
    ok = ok and text.splitlines()[1] == f".options {refs.OPTIONS}"
    ok = ok and refs.deck_options(rl_netlist({"op": {}})) == refs.OPTIONS
    return {"name": "inductor deck", "ok": ok}


def corner_check(refs):
    """An inductor deck's corners come from the netlist's jumps and contact edges, so a corner
    the deck never writes is still checked. The check exits when ngspice has no point at one."""
    tran = {"transient": {"stop_ps": 10000, "max_step_ps": 1000, "grid_ps": 1000}}
    netlist = {**rl_netlist(tran), "seed": 0}
    contact = {"name": "s1", "kind": "switch", "pins": {"a": "a", "b": "gnd"}, "initial": "open"}
    contact["events"] = [{"t_ps": 6000, "state": "closed"}, {"t_ps": 9500, "state": "open"}]
    contact["bounce"] = None
    netlist["parts"].append(contact)
    corners = refs.corner_times(netlist)
    ok = corners == [2000, 3000, 6000, 7000, 9500]
    refs.missed_corners("rl", corners, [0.0, 2e-9, 3e-9, 6e-9, 7e-9, 9.5e-9, 1e-8])
    try:
        refs.missed_corners("rl", corners, [0.0, 2e-9, 3e-9, 6e-9, 7.001e-9, 9.5e-9, 1e-8])
    except SystemExit as stop:
        ok = ok and str(stop) == "rl: ngspice has no point at corners [7000] ps of 5"
    else:
        ok = False
    return {"name": "corner check", "ok": ok}


def relay_deck(refs):
    """The coil as an inductor's lines with an ammeter, two hysteretic switches on the coil
    current's magnitude, the inductor deck's options, and the coil's `v` across its two pins.
    An operating point skips."""
    tran = {"transient": {"stop_ps": 60000000, "max_step_ps": 1000000, "grid_ps": 1000000}}
    netlist = rl_netlist(tran)
    netlist["nets"] += ["no", "nc"]
    netlist["parts"][1] = {
        "name": "k1",
        "kind": "relay",
        "pins": {"coil_a": "a", "coil_b": "gnd", "com": "a", "no": "no", "nc": "nc"},
        "inductance": {"value": 0.5},
        "coil_resistance": {"value": 70.0},
        "pull_in_current": {"value": 0.05},
        "drop_out_current": {"value": 0.01},
        "on_resistance": {"value": 0.1},
        "off_resistance": {"value": 1e8},
    }
    text, mapping = refs.translate("rly", netlist)
    want = [
        "* rly: generated by packages/engine/scripts/ngspice_refs.py from netlist.json. Do not edit.",
        f".options {refs.INDUCTOR_OPTIONS}",
        "rx_k1_s a x_k1 70.0",
        "lk1 x_k1 x_k1_c 0.5",
        "vx_k1_am x_k1_c 0 dc 0",
        "bx_k1_m x_k1_m 0 v = abs(i(vx_k1_am))",
        "sx_k1_no a no x_k1_m 0 swno_k1",
        ".model swno_k1 sw (vt=0.030000000000000002 vh=0.02 ron=0.1 roff=100000000.0)",
        "sx_k1_nc a nc x_k1_m 0 swnc_k1",
        ".model swnc_k1 sw (vt=0.030000000000000002 vh=0.02 ron=100000000.0 roff=0.1)",
    ]
    ok = text.splitlines()[: len(want)] == want
    ok = ok and "save v(a) v(no) v(nc) i(vv1) i(vx_k1_am)" in text.splitlines()
    ok = ok and mapping["i(vx_k1_am)"] == "#k1.i" and mapping["v(x_k1_m)"] is None
    ok = ok and refs.has_logic(netlist) and not refs.has_chip(netlist)
    values = {"#net.a": [5.0, 4.0], "#net.gnd": [0.0, 0.0], "#net.no": [5.0, 0.0], "#net.nc": [0.0, 4.0]}
    refs.part_voltages("rly", netlist, values, 2)
    ok = ok and values["#k1.v"] == [5.0, 4.0]
    try:
        refs.translate("rly", {**netlist, "analysis": {"op": {}}})
    except refs.Skip as skip:
        ok = ok and str(skip) == "a relay is translated only for a transient analysis"
    else:
        ok = False
    return {"name": "relay deck", "ok": ok}


def contact_edges(refs):
    """A contact closes above pull-in and opens at drop-out or below, for either sign, linear
    between points. A level between the two keeps the contact. A coil above pull-in at the
    first point starts closed."""
    times = [k * 1e-9 for k in range(9)]
    current = [0.0, 0.06, 0.03, 0.01, 0.03, -0.03, -0.07, -0.02, 0.0]
    got = refs.contact_edges(times, current, 0.05, 0.01)
    want = [
        {"level": "high", "t_ps": 833},
        {"level": "low", "t_ps": 3000},
        {"level": "high", "t_ps": 5500},
        {"level": "low", "t_ps": 7500},
    ]
    ok = got == want
    ok = ok and refs.contact_edges(times[:2], [0.06, 0.06], 0.05, 0.01) == []
    ok = ok and refs.contact_edges(times[:2], [0.06, 0.0], 0.05, 0.01) == [{"level": "low", "t_ps": 833}]
    # A 1 uH coil at 5 V moves 5 A inside the deck's 1 us cap, so one raw interval can reverse
    # from +100 mA to -100 mA. Both magnitudes sit above drop-out, and a magnitude chord emits
    # nothing. The signed watches open at +10 mA and close at -50 mA, which is what the engine
    # does on that current.
    ok = ok and refs.contact_edges([0.0, 1e-6], [0.1, -0.1], 0.05, 0.01) == [
        {"level": "low", "t_ps": 450000},
        {"level": "high", "t_ps": 750000},
    ]
    # The same interval ending at -20 mA opens once and stays open. -20 mA is below pull-in.
    ok = ok and refs.contact_edges([0.0, 1e-6], [0.1, -0.02], 0.05, 0.01) == [{"level": "low", "t_ps": 750000}]
    return {"name": "contact edges", "ok": ok}


def led_coil_without_marker(refs):
    """An LED across the coil, with no `rows-skipped` file, is not a bare fixture.

    The same directory with that file is bare.
    """
    parts = [
        {"kind": "relay", "name": "k1", "pins": {"coil_a": "vcc", "coil_b": "k"}},
        {"kind": "npn", "name": "q1", "pins": {"collector": "k", "base": "b", "emitter": "0"}},
        {"kind": "led", "name": "led1", "pins": {"anode": "vcc", "cathode": "k"}},
        {"kind": "resistor", "name": "r1"},
    ]
    netlist = {
        "analysis": {"transient": {"stop_ps": 10, "max_step_ps": 1, "grid_ps": 1}},
        "parts": parts,
    }
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp)
        (path / "netlist.json").write_text(json.dumps(netlist))
        unmarked = not refs.bare_fixture(path)
        (path / refs.ROWS_SKIPPED_FILE).write_text("ringing at about 9 us sits on a 50 us grid\n")
        marked = refs.bare_fixture(path)
    return unmarked and marked


def bare_flyback(refs):
    """`p-relay-npn-bare` has `rows-skipped`, so its reference keeps three numbers.

    An example keeps its rows. An LED across the coil keeps its rows when the
    file is absent. The summary reads the collector extreme and the one release
    from ngspice's own points.
    """
    fixture = Path("packages/engine/circuit/tests/fixtures/p-relay-npn-bare")
    ok = refs.bare_fixture(fixture)
    ok = ok and refs.rows_skipped_reason(fixture) == "ringing at about 9 us sits on a 50 us grid"
    ok = ok and not refs.bare_fixture(Path("packages/engine/tests/circuits/relay-switch"))
    ok = ok and led_coil_without_marker(refs)
    times = [0.0, 1e-6, 2e-6, 3e-6]
    current = [0.0, 0.06, 0.02, 0.0]
    voltage = [1.0, 5.0, -2.0, 3.0]
    got = refs.bare_summary(times, voltage, current, 0.05, 0.01)
    ok = ok and got == {
        "peak": 5.0,
        "minimum": -2.0,
        "t_ps": 2_500_000,
        "span": 0.06,
        "t_s": [2e-6, 3e-6],
        "i": [0.02, 0.0],
    }
    deck = "save v(a) v(b) i(vx_k1_am)\ntran 1p 10p 0 1p\n"
    ok = ok and refs.narrow_save(deck, "v(k)", "i(vx_k1_am)") == "save v(k) i(vx_k1_am)\ntran 1p 10p 0 1p\n"
    try:
        refs.narrow_save("tran 1p 10p\n", "v(k)", "i(vx_k1_am)")
    except SystemExit as stop:
        ok = ok and str(stop) == "a bare flyback deck has 0 save lines"
    else:
        ok = False
    try:
        refs.bare_summary([0.0, 1e-6], [1.0, 1.0], [0.06, 0.06], 0.05, 0.01)
    except SystemExit as stop:
        ok = ok and str(stop) == "a bare flyback has 0 release edges"
    else:
        ok = False
    return {"name": "bare flyback", "ok": ok}


def vcd_reader(refs):
    """The VCD the Rust unit test in packages/engine/core/src/logic.rs pins, read back."""
    text = """$timescale 1 ps $end
$scope module u1 $end
$var wire 1 ! out $end
$var wire 1 " trig $end
$upscope $end
$scope module u2 $end
$var wire 1 # out $end
$upscope $end
$enddefinitions $end
#0
$dumpvars
1!
0"
0#
$end
#231
1"
#626
0!
1#
"""
    edges = refs.read_vcd(text)
    ok = edges == {
        "#u1.logic.out": {"initial": "high", "edges": [{"t_ps": 626, "level": "low"}]},
        "#u1.logic.trig": {"initial": "low", "edges": [{"t_ps": 231, "level": "high"}]},
        "#u2.logic.out": {"initial": "low", "edges": [{"t_ps": 626, "level": "high"}]},
    }
    one = {"count": 1, "first_ps": 626, "last_ps": 626}
    ok = ok and refs.summary(edges) == {
        "#u1.logic.out": {"initial": "high", "final": "low", **one},
        "#u1.logic.trig": {"initial": "low", "final": "high", "count": 1, "first_ps": 231, "last_ps": 231},
        "#u2.logic.out": {"initial": "low", "final": "high", **one},
    }
    quiet = {"#u1.logic.reset": {"initial": "high", "edges": []}}
    ok = ok and refs.summary(quiet) == {
        "#u1.logic.reset": {"initial": "high", "final": "high", "count": 0, "first_ps": None, "last_ps": None},
    }
    return {"name": "vcd reader", "ok": ok}
