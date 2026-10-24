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
            "name": "reserved transistor prefix at the operating point",
            "nets": ["gnd", "a", "x_q1_c"],
            "part": bjt("q1", "npn", "x_q1_c", "a", "gnd"),
            "commit": [],
            "exit": 0,
            "reason": "a deck with a transistor reserves names starting with x_",
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
    cases.append(transistor_deck(refs))
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
                    "events": [
                        {"t_ps": 2000, "baud": 500000000, "frame": "8N1", "bytes": [0]}
                    ],
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
        "events": [
            {"t_ps": 5000000000, "baud": 9600, "frame": "8N1", "bytes": [0x30, 0x68, 0x69]}
        ],
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
    card = (
        "is=1.0 bf=2.0 br=3.0 nf=4.0 nr=5.0 vaf=6.0 rb=7.0 rc=8.0 re=9.0 cje=10.0 cjc=11.0 "
        "mje=0 mjc=0"
    )
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
