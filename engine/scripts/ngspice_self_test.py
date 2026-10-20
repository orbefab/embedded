"""Self-test cases for scripts/ngspice_refs.py. No ngspice."""

import json
import tempfile
from pathlib import Path

from bounce import bounce_vectors


def self_test(refs):
    """Skip paths, with no ngspice. stdout is one JSON document."""
    specs = [
        {
            "name": "unknown kind",
            "part": {"name": "l1", "kind": "inductor", "pins": {"a": "a", "b": "gnd"}},
            "commit": [],
            "exit": 0,
            "reason": "kind inductor has no translation",
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
            "part": {"name": "l1", "kind": "inductor", "pins": {"a": "a", "b": "gnd"}},
            "commit": ["ngspice.json"],
            "exit": 1,
            "reason": "kind inductor has no translation",
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
    cases.append(potentiometer_deck(refs))
    cases.append(bounce_vectors())
    cases.append(vcd_reader(refs))
    document = {"ok": all(case["ok"] for case in cases), "cases": cases}
    print(json.dumps(document, indent=2))
    return 0 if document["ok"] else 1


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
        f"vbp{k} bp{k} 0 pwl({at}p 0 {at + 1}p 0)"
        for k, at in enumerate((2000, 3000, 4000, 5000, 7000, 8000, 9000))
    ]
    cases = [
        (deck(op, "voltage_source", "voltage", {"value": 5.0}), ["vs1 a 0 dc 5.0", "save all"]),
        (
            deck(op, "current_source", "current", {"value": 0.001}),
            ["is1 0 a dc 0.001", "save all @is1[current]"],
        ),
        (deck(op, "voltage_source", "voltage", step), ["vs1 a 0 dc 0.0", "save all"]),
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
            deck(tran, "current_source", "current", pulse),
            [
                "is1 0 a dc 0.5 pwl(0p 0.5 2000p 0.5 3000p 1.0 4000p 1.0 5000p 0.5 7000p 0.5 "
                "8000p 1.0 9000p 1.0)",
                *corners,
                "save v(a) @is1[current]",
            ],
        ),
    ]
    ok = all(lines == want for (lines, _), want in cases)
    ok = ok and cases[1][0][1]["i(@is1[current])"] == refs.Negated("#s1.i")
    return {"name": "source decks", "ok": ok}


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
    return {"name": "potentiometer deck", "ok": ok}


def vcd_reader(refs):
    """The VCD the Rust unit test in crates/core/src/logic.rs pins, read back."""
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
