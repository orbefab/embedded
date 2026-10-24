# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Write each example's ngspice reference from its netlist.json.

    uv run scripts/ngspice_refs.py [--check] [examples/<name> | crates/cli/tests/fixtures/<name> ...]
    uv run scripts/ngspice_refs.py --self-test

For every example it translates netlist.json into deck.cir, runs `ngspice -b`
on it, and writes the operating point, or the transient waveforms, to
ngspice.json. A chip's latch edges go to tran.logic. With no example arguments
it takes every examples/*/netlist.json, then every
crates/cli/tests/fixtures/*/netlist.json, the circuits only the ngspice test
runs. `--check` regenerates in memory and
exits 1 if any committed byte differs, or if a skipped example already has
deck.cir or ngspice.json. For an example with a chip it also runs the engine and exits 1
unless the run's logic.vcd reads back as its observations.logic summary. stdout is one
JSON document.

A part with no kind, an unknown kind, a kind that runs firmware, a part with no
pins, or a missing pin raises Skip. The example `npn-blinker` raises Skip too,
because ngspice aborts that deck at the deck's options with Timestep too small.
The script exits when ngspice's log shows the operating point left the plain
Newton iteration, and `--check` exits the same way. The message names the
example and ngspice's lines. `--self-test` checks the skip paths, the source
deck lines, the bounce test vectors, the VCD reader and its summary, and that
refusal. The refusal case runs ngspice. The rest do not. Without ngspice-42
on PATH, `--self-test` prints one failed case and exits 1.
"""

import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

from bounce import switch_edges

ROOT = Path(__file__).resolve().parent.parent
FORMAT = 0
OPTIONS = "temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12"
RAW = "op.raw"
TRAN_RAW = "tran.raw"
EDGES_RAW = "edges.raw"
CHIPS = ("ne555",)
FIRMWARE = ("atmega328p",)
TRANSISTORS = ("npn", "pnp", "nmos")
BJT_KINDS = ("npn", "pnp")
# Each BJT field's .model parameter. `mje=0 mjc=0` makes the depletion capacitances constant.
BJT_CARD = (
    ("is", "saturation_current"),
    ("bf", "forward_beta"),
    ("br", "reverse_beta"),
    ("nf", "forward_emission"),
    ("nr", "reverse_emission"),
    ("vaf", "early_voltage"),
    ("rb", "base_resistance"),
    ("rc", "collector_resistance"),
    ("re", "emitter_resistance"),
    ("cje", "base_emitter_capacitance"),
    ("cjc", "base_collector_capacitance"),
)
# Level 1 parameters the netlist names. `l=w` and `is=0` are written on the device line and the card.
# Capacitances are plain elements, so a default `tox` cannot add Meyer caps.
MOS_CARD = (
    ("vto", "threshold_voltage"),
    ("kp", "transconductance_parameter"),
    ("lambda", "channel_length_modulation"),
)
MOS_CAPS = (
    ("gs", "gate_source_capacitance"),
    ("gd", "gate_drain_capacitance"),
    ("ds", "drain_source_capacitance"),
)
# A chip deck caps ngspice's step at stop/400000 and linearizes the waveform onto stop/2000.
CHIP_TMAX = 400_000
CHIP_GRID = 2_000
# The engine's conductance from every net to ground.
ENGINE_GMIN = 1e-12
# The engine's first step after a contact edge or a source jump. The Rust test pins both.
EDGE_RAMP_PS = 1000
# Knots strictly before the stop, including the one at t = 0. This bounds the
# deck's size. The engine's rows come from its grid, not from the knots, so a
# skipped deck can still be a run that finishes.
KNOT_CAP = 100_000
# A transient with more saved points than this stores the engine's grid and
# each column's span over ngspice's own points. The largest dense reference is
# the nmos-pulse fixture at 2745 points. docs/reference.md names this number.
POINT_CAP = 3000
# ngspice-42 prints these in cktop.c and optran.c only after the plain Newton
# operating point failed or was skipped. The `noopiter` help text is not a log line.
OP_FALLBACKS = (
    "Starting dynamic gmin stepping",
    "Dynamic gmin stepping failed",
    "Starting true gmin stepping",
    "True gmin stepping failed",
    "Starting spice3 gmin stepping",
    "spice3 gmin stepping failed",
    "Starting source stepping",
    "source stepping failed",
    "Transient op started",
    "Transient op failed",
    "The operating point could not be simulated successfully.",
)


class Skip(Exception):
    """The netlist has something this translator does not write."""


@dataclass(frozen=True)
class Negated:
    """A vector whose ref reads the other way round, such as a current source's current."""

    ref: str


@dataclass(frozen=True)
class GateCurrent:
    """A MOSFET gate ammeter. It is not an engine column. Pin power reads it."""

    ref: str


def main(argv):
    if "--self-test" in argv:
        from ngspice_self_test import self_test

        return self_test(sys.modules[__name__])
    check = "--check" in argv
    targets = [Path(a) for a in argv if a != "--check"]
    if not targets:
        targets = [
            p.parent
            for folder in ("examples", "crates/cli/tests/fixtures")
            for p in sorted((ROOT / folder).glob("*/netlist.json"))
        ]
    report = report_examples(targets, check, ngspice_version())
    print(json.dumps(report, indent=2))
    return exit_code(report)


def report_examples(targets, check, version):
    report = {"ngspice": version, "checked" if check else "written": [], "skipped": []}
    if check:
        report["drift"] = []
        report["logic"] = []
    for example in targets:
        example = example.resolve()
        label = label_of(example)
        try:
            files = generate(example, version)
        except Skip as skip:
            entry = {"example": label, "reason": str(skip)}
            committed = [n for n in ("deck.cir", "ngspice.json") if (example / n).is_file()]
            if check and committed:
                verb = "was" if len(committed) == 1 else "were"
                entry["failure"] = f"committed {' and '.join(committed)} {verb} not checked"
            report["skipped"].append(entry)
            continue
        if check:
            for name, text in files.items():
                path = example / name
                if not path.is_file() or path.read_bytes() != text.encode():
                    report["drift"].append(f"{label}/{name}")
            report["checked"].append(label)
            if has_chip(json.loads((example / "netlist.json").read_bytes())):
                report["logic"].append({"example": label, "ok": logic_reads_back(example)})
        else:
            for name, text in files.items():
                write_atomic(example / name, text)
            report["written"].append(label)
    return report


def label_of(example):
    try:
        return str(example.relative_to(ROOT))
    except ValueError:
        return example.as_posix()


def exit_code(report):
    unchecked = any("failure" in entry for entry in report["skipped"])
    unread = any(not entry["ok"] for entry in report.get("logic", []))
    return 1 if report.get("drift") or unchecked or unread else 0


def has_chip(netlist):
    return any(part.get("kind") in CHIPS for part in netlist["parts"])


def logic_reads_back(example):
    """Runs the engine on the example and reads its logic.vcd back as observations.logic."""
    subprocess.run(
        ["cargo", "build", "--release", "--locked", "--quiet", "-p", "sim-cli"],
        cwd=ROOT,
        check=True,
    )
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp, "run")
        subprocess.run(
            [ROOT / "target/release/sim", "run", example / "netlist.json", "--out", out],
            capture_output=True,
            check=True,
        )
        observations = json.loads((out / "observations.json").read_bytes())
        return summary(read_vcd((out / "traces/logic.vcd").read_text())) == observations["logic"]


def summary(signals):
    """observations.logic from `read_vcd`'s edges: each signal's levels, count and times."""
    out = {}
    for ref, signal in signals.items():
        edges = signal["edges"]
        out[ref] = {
            "initial": signal["initial"],
            "final": edges[-1]["level"] if edges else signal["initial"],
            "count": len(edges),
            "first_ps": edges[0]["t_ps"] if edges else None,
            "last_ps": edges[-1]["t_ps"] if edges else None,
        }
    return out


def read_vcd(text):
    """`{ref: {initial, edges: [{t_ps, level}]}}` from a logic.vcd."""
    tokens = text.split()
    refs = {}
    scope = None
    signals = {}
    at = None
    k = 0
    while k < len(tokens):
        token = tokens[k]
        if token == "$scope":
            scope = tokens[k + 2]
            k += 4
        elif token == "$upscope":
            scope = None
            k += 2
        elif token == "$var":
            ident, name = tokens[k + 3], tokens[k + 4]
            refs[ident] = f"#{scope}.logic.{name}"
            k += 6
        elif token == "$timescale":
            k = tokens.index("$end", k) + 1
        elif token in ("$enddefinitions", "$dumpvars", "$end"):
            k += 1
        elif token.startswith("#"):
            at = int(token[1:])
            k += 1
        else:
            level = {"0": "low", "1": "high"}[token[0]]
            ref = refs[token[1:]]
            if ref not in signals:
                signals[ref] = {"initial": level, "edges": []}
            else:
                signals[ref]["edges"].append({"t_ps": at, "level": level})
            k += 1
    return signals


def ngspice_version():
    out = subprocess.run(["ngspice", "-v"], capture_output=True, text=True, check=True).stdout
    match = re.search(r"ngspice-\d+", out)
    if not match:
        raise SystemExit(f"no version in `ngspice -v` output: {out!r}")
    return match.group(0)


def generate(example, version):
    netlist_bytes = (example / "netlist.json").read_bytes()
    netlist = json.loads(netlist_bytes)
    deck, mapping = translate(example.name, netlist)
    transient = "transient" in netlist["analysis"]
    chips = [part for part in netlist["parts"] if part["kind"] in CHIPS]
    raws = [TRAN_RAW, EDGES_RAW] if chips else [TRAN_RAW if transient else RAW]
    vectors, *edges = run_ngspice(deck, raws, label_of(example))
    values = {}
    gate = {}
    for variable, column in vectors.items():
        if transient and variable == "time":
            continue
        if variable not in mapping:
            raise SystemExit(f"{example.name}: unexpected ngspice vector {variable}")
        ref = mapping[variable]
        if ref is None:
            continue
        if isinstance(ref, Negated):
            values[ref.ref] = [-value for value in column]
        elif isinstance(ref, GateCurrent):
            gate[ref.ref] = column
        else:
            values[ref] = column
    if transient:
        count = len(vectors["time"])
        values[f"#net.{netlist['ground']}"] = [0.0] * count
        part_voltages(example.name, netlist, values, count)
        times, spans = vectors["time"], None
        if store_on_engine_grid(count, bool(chips)):
            spans = dense_spans(netlist, values, gate)
            times, values, gate = on_engine_grid(netlist, times, values, gate)
    reference = {
        "format": FORMAT,
        "netlist_sha256": hashlib.sha256(netlist_bytes).hexdigest(),
        "deck_sha256": hashlib.sha256(deck.encode()).hexdigest(),
        "ngspice": version,
        "options": OPTIONS,
    }
    if transient:
        reference["tran"] = {
            "edge_ramp_ps": EDGE_RAMP_PS,
            "jumps": source_jumps(netlist),
            "switches": switches(netlist),
            "t_s": times,
            "values": values,
        }
        if gate:
            reference["tran"]["gate"] = gate
        if spans:
            reference["tran"]["span"] = spans
        if chips:
            stop = netlist["analysis"]["transient"]["stop_ps"]
            reference["tran"]["grid_ps"] = stop // CHIP_GRID
            reference["tran"]["logic"] = {
                f"#{part['name']}.logic.out": crossings(
                    edges[0]["time"], edges[0][f"v(x_{part['name'].lower()}_q)"]
                )
                for part in chips
            }
    else:
        reference["op"] = op_point(netlist, values)
    text = json.dumps(reference, indent=2, sort_keys=True) + "\n"
    return {"deck.cir": deck, "ngspice.json": text}


def crossings(times, latch):
    """Each 0.5 crossing of a latch node, linear between ngspice's points, to the picosecond."""
    edges = []
    for (t0, v0), (t1, v1) in zip(zip(times, latch), zip(times[1:], latch[1:])):
        if (v0 - 0.5) * (v1 - 0.5) < 0:
            at = t0 + (0.5 - v0) * (t1 - t0) / (v1 - v0)
            edges.append({"level": "high" if v1 > 0.5 else "low", "t_ps": round(at * 1e12)})
    return edges


def op_point(netlist, values):
    op = {ref: column[0] for ref, column in values.items()}
    op[f"#net.{netlist['ground']}"] = 0.0
    for part in netlist["parts"]:
        if part["kind"] in NO_QUANTITIES:
            continue
        name = part["name"]
        if part["kind"] in BJT_KINDS:
            c, b, e = (net_voltage(op, net) for net in pin_nets(part))
            op[f"#{name}.p"] = (c - e) * op[f"#{name}.ic"] + (b - e) * op[f"#{name}.ib"]
            continue
        if part["kind"] == "nmos":
            d, _g, s = (net_voltage(op, net) for net in pin_nets(part))
            op[f"#{name}.p"] = (d - s) * op[f"#{name}.id"]
            continue
        first, second = (net_voltage(op, net) for net in pin_nets(part))
        op[f"#{name}.v"] = first - second
    return op


def part_voltages(label, netlist, values, count):
    """Each two-pin part's `.v` on every transient row, from its pin-order node voltages."""
    for part in netlist["parts"]:
        if part["kind"] in NO_QUANTITIES or part["kind"] in TRANSISTORS:
            continue
        try:
            first, second = (values[f"#net.{net}"] for net in pin_nets(part))
        except KeyError as missing:
            raise SystemExit(f"{label}: no node voltage {missing}") from None
        if len(first) != count or len(second) != count:
            raise SystemExit(f"{label}: {part['name']} voltage has the wrong length")
        values[f"#{part['name']}.v"] = [a - b for a, b in zip(first, second)]


def source_jumps(netlist):
    """Every source jump before the stop, in part order, from `jumps`."""
    stop = netlist["analysis"]["transient"]["stop_ps"]
    times = []
    for part in netlist["parts"]:
        for key in ("voltage", "current"):
            wave = part.get(key)
            if not isinstance(wave, dict) or "kind" not in wave:
                continue
            for at, _old, _new in jumps(wave):
                if at >= stop:
                    break
                times.append(at)
    return times


def switches(netlist):
    """Every contact edge before the stop, per switch ref."""
    stop = netlist["analysis"]["transient"]["stop_ps"]
    return {
        f"#{part['name']}": [
            edge for edge in switch_edges(netlist["seed"], part) if edge["t_ps"] < stop
        ]
        for part in netlist["parts"]
        if part["kind"] == "switch"
    }


PIN_ORDER = {
    "resistor": ("a", "b"),
    "potentiometer": ("a", "wiper", "b"),
    "voltage_source": ("p", "n"),
    "current_source": ("p", "n"),
    "diode": ("anode", "cathode"),
    "led": ("anode", "cathode"),
    "npn": ("collector", "base", "emitter"),
    "pnp": ("collector", "base", "emitter"),
    "nmos": ("drain", "gate", "source"),
    "capacitor": ("a", "b"),
    "switch": ("a", "b"),
    "ne555": ("gnd", "trig", "out", "reset", "cont", "thres", "disch", "vcc"),
}

NO_QUANTITIES = (*CHIPS, "potentiometer")


def pin_nets(part):
    if "kind" not in part:
        raise Skip("part has no kind")
    kind = part["kind"]
    if kind in FIRMWARE:
        raise Skip(f"kind {kind} runs firmware, which a deck cannot")
    try:
        order = PIN_ORDER[kind]
    except KeyError:
        raise Skip(f"kind {kind} has no translation") from None
    if "pins" not in part:
        raise Skip(f"kind {kind} has no pins")
    pins = part["pins"]
    try:
        return [pins[pin] for pin in order]
    except KeyError as missing:
        name = missing.args[0] if missing.args else "pins"
        raise Skip(f"kind {kind} is missing pin {name}") from None
    except TypeError:
        raise Skip(f"kind {kind} has no pins") from None


def net_voltage(op, net):
    return op[f"#net.{net}"]


def translate(title, netlist):
    """The deck text, and a map from each ngspice vector name to its ref."""
    if title == "npn-blinker":
        raise Skip("ngspice aborts this deck at the deck's options with Timestep too small")
    try:
        return translate_shape(title, netlist)
    except (KeyError, TypeError, AttributeError) as error:
        # The engine's parser is the validator; anything it would reject is skipped here.
        raise Skip(f"netlist.json is not a netlist the engine accepts ({type(error).__name__})") from None


def translate_shape(title, netlist):
    ground = netlist["ground"]
    nets = netlist["nets"]
    parts = sorted(netlist["parts"], key=lambda p: p["name"])
    transient = netlist["analysis"].get("transient")
    # SPICE names are case-insensitive, and ngspice reads `gnd` as ground.
    for kind, names in (("net", nets), ("part", [p["name"] for p in parts])):
        lowered = [n.lower() for n in names]
        if len(set(lowered)) != len(lowered):
            raise Skip(f"two {kind} names are equal ignoring case")
    if any(n.lower() == "gnd" and n != ground for n in nets):
        raise Skip("a net other than ground is named gnd")
    names = nets + [p["name"] for p in parts]
    if transient and any(n.lower().startswith(("ctl_", "bp", "x_")) for n in names):
        raise Skip("a transient deck reserves names starting with ctl_, bp or x_")
    transistors = any(p.get("kind") in TRANSISTORS for p in parts)
    if transistors and any(n.lower().startswith("x_") for n in names):
        raise Skip("a deck with a transistor reserves names starting with x_")

    def node(net):
        return "0" if net == ground else net.lower()

    mapping = {f"v({net.lower()})": f"#net.{net}" for net in nets if net != ground}
    lines = [
        f"* {title}: generated by scripts/ngspice_refs.py from netlist.json. Do not edit.",
        f".options {OPTIONS}",
    ]
    saves = []
    corners = set()
    latches = []
    for part in parts:
        pin_list = pin_nets(part)
        if any(net not in nets for net in pin_list):
            raise Skip(f"part {part['name']} pins a net that is not declared")
        name = part["name"].lower()
        kind = part["kind"]
        pins = " ".join(node(net) for net in pin_list)
        ref = f"#{part['name']}.i"
        if kind == "resistor":
            lines.append(f"r{name} {pins} {number(part['resistance'])}")
            saves.append(f"@r{name}[i]")
            mapping[f"i(@r{name}[i])"] = ref
        elif kind == "potentiometer":
            a, wiper, b = (node(net) for net in pin_list)
            position = part["position"]
            if position.get("kind") == "steps":
                if position["events"]:
                    raise Skip("a moving potentiometer has one position in a deck")
                position = position["initial"]
            position = float(position["value"])
            ohms = float(part["resistance"]["value"])
            lines.append(f"r{name}_a {a} {wiper} {position * ohms!r}")
            lines.append(f"r{name}_b {wiper} {b} {(1.0 - position) * ohms!r}")
        elif kind == "voltage_source":
            lines.append(f"v{name} {pins} {source_value(part['voltage'], transient, corners)}")
            mapping[f"i(v{name})"] = ref
        elif kind == "current_source":
            # The engine's current leaves at `p`, and ngspice's flows from its first node
            # through the source to its second. ngspice reports the value it drives.
            n, p = (node(net) for net in reversed(pin_list))
            lines.append(f"i{name} {n} {p} {source_value(part['current'], transient, corners)}")
            saves.append(f"@i{name}[current]")
            mapping[f"i(@i{name}[current])"] = Negated(ref)
        elif kind in ("diode", "led"):
            lines.append(f"d{name} {pins} dmod_{name}")
            params = (
                f"is={number(part['saturation_current'])} "
                f"n={number(part['emission_coefficient'])} "
                f"rs={number(part['series_resistance'])}"
            )
            lines.append(f".model dmod_{name} d ({params})")
            saves.append(f"@d{name}[id]")
            mapping[f"i(@d{name}[id])"] = ref
        elif kind in BJT_KINDS:
            # A 0 V source in series with the collector and the base reads each whole pin current.
            c, b, e = (node(net) for net in pin_list)
            for pin, at in (("c", c), ("b", b)):
                lines.append(f"vx_{name}_{pin} {at} x_{name}_{pin} dc 0")
                saves.append(f"i(vx_{name}_{pin})")
                mapping[f"i(vx_{name}_{pin})"] = f"#{part['name']}.i{pin}"
                mapping[f"v(x_{name}_{pin})"] = None
            lines.append(f"q{name} x_{name}_c x_{name}_b {e} qmod_{name}")
            params = " ".join(f"{key}={number(part[field])}" for key, field in BJT_CARD)
            lines.append(f".model qmod_{name} {kind} ({params} mje=0 mjc=0)")
        elif kind == "nmos":
            # Ammeters read the whole drain and gate pin currents, capacitors included.
            # The three capacitances are plain elements on the device side of the ammeters.
            # `l=w` so `kp` alone sets the current. `is=0` leaves no body diode.
            d, g, s = (node(net) for net in pin_list)
            for pin, at in (("d", d), ("g", g)):
                lines.append(f"vx_{name}_{pin} {at} x_{name}_{pin} dc 0")
                saves.append(f"i(vx_{name}_{pin})")
                mapping[f"v(x_{name}_{pin})"] = None
            mapping[f"i(vx_{name}_d)"] = f"#{part['name']}.id"
            mapping[f"i(vx_{name}_g)"] = GateCurrent(f"#{part['name']}")
            lines.append(f"m{name} x_{name}_d x_{name}_g {s} {s} mmod_{name} l=100u w=100u")
            params = " ".join(f"{key}={number(part[field])}" for key, field in MOS_CARD)
            lines.append(f".model mmod_{name} nmos (level=1 {params} is=0)")
            cap_nodes = {
                "gs": (f"x_{name}_g", s),
                "gd": (f"x_{name}_g", f"x_{name}_d"),
                "ds": (f"x_{name}_d", s),
            }
            for suffix, field in MOS_CAPS:
                a, b = cap_nodes[suffix]
                lines.append(f"c{name}_{suffix} {a} {b} {number(part[field])}")
        elif kind == "capacitor":
            lines.append(f"c{name} {pins} {number(part['capacitance'])}")
            if part["initial"] is not None:
                lines.append(initial_condition(part, ground, node, transient))
            saves.append(f"@c{name}[i]")
            mapping[f"i(@c{name}[i])"] = ref
        elif kind == "switch":
            if not transient:
                raise Skip("a switch is translated only for a transient analysis")
            lines.append(f"s{name} {pins} ctl_{name} 0 swmod_{name}")
            lines.append(
                f".model swmod_{name} sw (vt=0.5 vh=0 "
                f"ron={number(part['on_resistance'])} roff={number(part['off_resistance'])})"
            )
            edges = switches({"analysis": netlist["analysis"], "seed": netlist["seed"], "parts": [part]})
            level = 1 if part["initial"] == "closed" else 0
            pwl = [(0, level)]
            for edge in edges[f"#{part['name']}"]:
                at = edge["t_ps"]
                pwl.append((at, level))
                level = 1 if edge["state"] == "closed" else 0
                pwl.append((at + EDGE_RAMP_PS, level))
                corners.update((at, at + EDGE_RAMP_PS))
            text = " ".join(f"{at}p {value}" for at, value in pwl)
            lines.append(f"vctl_{name} ctl_{name} 0 pwl({text})")
            saves.append(f"@s{name}[i]")
            mapping[f"i(@s{name}[i])"] = ref
        elif kind == "ne555":
            if not transient:
                raise Skip("a chip is translated only for a transient analysis")
            lines += ne555_lines(name, part, node)
            latches.append(f"v(x_{name}_q)")
        else:
            raise Skip(f"kind {kind} has no translation")
    if latches:
        lines.append(
            "* The engine's gmin on every node. A chip's ladder holds CONT flat, with no span to hide it."
        )
        shunted = [node(net) for net in nets if net != ground] + [
            f"x_{p['name'].lower()}_lo" for p in parts if p["kind"] in CHIPS
        ]
        lines += [f"rx_gmin_{n} {n} 0 {1 / ENGINE_GMIN!r}" for n in shunted]
    elif transistors:
        lines.append("* The engine's gmin on every node, beside the gmin ngspice puts across each junction.")
        shunted = [node(net) for net in nets if net != ground]
        lines += [f"rx_gmin_{n} {n} 0 {1 / ENGINE_GMIN!r}" for n in shunted]
    if not transient:
        reject_duplicate_devices(lines)
        lines += [
            ".control",
            " ".join(["save all", *saves]),
            "op",
            "set filetype=ascii",
            f"write {RAW} all",
            ".endc",
            ".end",
        ]
        return "\n".join(lines) + "\n", mapping
    if corners:
        lines.append(
            "* One source per ramp corner makes it a breakpoint. ngspice skips a PWL corner it lands on an ulp early."
        )
    for k, at in enumerate(sorted(corners)):
        lines.append(f"vbp{k} bp{k} 0 pwl({at}p 0 {at + 1}p 0)")
    reject_duplicate_devices(lines)
    stop = transient["stop_ps"]
    voltages = [f"v({net.lower()})" for net in nets if net != ground]
    sources = [f"i(v{p['name'].lower()})" for p in parts if p["kind"] == "voltage_source"]
    lines += [".control", " ".join(["save", *voltages, *sources, *saves, *latches])]
    if latches:
        # The latch edges keep ngspice's own points. The waveform is linearized, which keeps it
        # small: the cap that resolves the edges makes some 400000 points.
        lines += [
            f"tran {stop // CHIP_GRID}p {stop}p 0 {stop // CHIP_TMAX}p",
            "set filetype=ascii",
            " ".join(["write", EDGES_RAW, *latches]),
            " ".join(["linearize", *voltages, *sources, *saves]),
        ]
    else:
        lines += [
            # ngspice picks its own points under this cap, which does not depend on the engine's max_step.
            f"tran {stop // 1000}p {stop}p 0 {stop // 1000}p",
            "set filetype=ascii",
        ]
    lines += [f"write {TRAN_RAW}", ".endc", ".end"]
    return "\n".join(lines) + "\n", mapping


def reject_duplicate_devices(lines):
    names = [
        line.split(None, 1)[0].lower()
        for line in lines
        if line and line[0] not in ".*"
    ]
    if len(set(names)) != len(names):
        raise Skip("two device names are equal ignoring case")


def ne555_lines(name, part, node):
    """The engine's NE555 as B-sources. Each comparator reads high strictly above its threshold,
    as the engine's watches do. The latch state is a node held on 1 ohm / 1 pF."""
    gnd, trig, out, reset, cont, thres, disch, vcc = (
        node(net) for net in pin_nets(part)
    )
    q = f"x_{name}_q"
    lower = f"x_{name}_lo"
    ladder = number(part["ladder_resistance"])
    discharge, output = part["discharge"], part["output"]
    high = f"V({q}) > 0.5"
    latch = (
        f"(V({reset},{gnd}) > {number(part['reset_threshold'])}) ? "
        f"((V({trig},{lower}) > 0) ? ((V({thres},{cont}) > 0) ? 0 : ({high} ? 1 : 0)) : 1) : 0"
    )
    return [
        f"rx_{name}_l1 {vcc} {cont} {ladder}",
        f"rx_{name}_l2 {cont} {lower} {ladder}",
        f"rx_{name}_l3 {lower} {gnd} {ladder}",
        f"rx_{name}_sup {vcc} {gnd} {number(part['supply_resistance'])}",
        f"* {name} latch: RESET overrides TRIG, TRIG overrides THRES (SLFS022K Table 6-1).",
        f"bx_{name}_s x_{name}_s 0 v = {latch}",
        f"rx_{name}_q x_{name}_s {q} 1",
        f"cx_{name}_q {q} 0 1e-12",
        f"bx_{name}_oh {vcc} {out} i = ({high}) ? "
        f"(V({vcc},{out}) - {number(output['high_drop'])}) / {number(output['high_resistance'])} : 0",
        f"bx_{name}_ol {out} {gnd} i = ({high}) ? 0 : "
        f"(V({out},{gnd}) - {number(output['low_drop'])}) / {number(output['low_resistance'])}",
        f"bx_{name}_d {disch} {gnd} i = V({disch},{gnd}) / (({high}) ? "
        f"{number(discharge['off_resistance'])} : {number(discharge['on_resistance'])})",
    ]


def initial_condition(part, ground, node, transient):
    """`.ic` holds the node at the operating point before `tran`, as the engine holds `initial`."""
    if not transient:
        raise Skip("a held capacitor is translated only for a transient analysis")
    a, b = (part["pins"][pin] for pin in PIN_ORDER["capacitor"])
    volts = float(part["initial"]["value"])
    if b == ground and a != ground:
        return f".ic v({node(a)})={volts!r}"
    if a == ground and b != ground:
        return f".ic v({node(b)})={-volts!r}"
    raise Skip("a held capacitor needs exactly one pin on ground")


def number(value):
    return repr(float(value["value"]))


def source_value(value, transient, corners):
    """A source's value on its deck line. A waveform holds its idle level at the operating
    point, and in a transient each jump ramps over EDGE_RAMP_PS. Its knots in (0, stop] are
    corners. A serial line idles at `mark`."""
    if "kind" not in value:
        return f"dc {number(value)}"
    if value["kind"] == "serial" and not value["events"]:
        return f"dc {number(value['mark'])}"
    if value["kind"] == "steps" and not value["events"]:
        return f"dc {number(value['initial'])}"
    held = value["mark"] if value["kind"] == "serial" else value["initial"]
    initial = f"dc {number(held)}"
    if not transient:
        return initial
    points = knots(value, transient["stop_ps"])
    corners.update(at for at, _ in points if 0 < at <= transient["stop_ps"])
    return initial + " pwl(" + " ".join(f"{at}p {level!r}" for at, level in points) + ")"


def knots(wave, stop):
    """The idle level at 0, then each jump from its old level at the jump to its new level
    EDGE_RAMP_PS later, through the first knot at or past `stop`."""
    points = [(0, idle_level(wave))]
    before = 1 if 0 < stop else 0
    for at, old, new in jumps(wave):
        for point in ((at, old), (at + EDGE_RAMP_PS, new)):
            if point[0] < stop:
                before += 1
                if before > KNOT_CAP:
                    raise Skip(f"a waveform has more than {KNOT_CAP} knots before the stop")
            points.append(point)
            if point[0] >= stop:
                return points
    return points


def idle_level(wave):
    """The level at t = 0. A serial line idles at `mark`."""
    if wave["kind"] == "serial":
        return float(wave["mark"]["value"])
    return float(wave["initial"]["value"])


def serial_bits(data):
    """Start at space, eight data bits least significant first, stop at mark."""
    for byte in data:
        yield False
        for shift in range(8):
            yield (byte >> shift) & 1 == 1
        yield True


def serial_jumps(wave):
    """Each 8N1 jump. A boundary is `t_ps + floor(i * 10**12 / baud)` from that event."""
    mark = float(wave["mark"]["value"])
    space = float(wave["space"]["value"])
    level = mark
    span = 1_000_000_000_000
    for event in wave["events"]:
        if event["frame"] != "8N1":
            raise Skip(f"frame {event['frame']} is not 8N1")
        baud = event["baud"]
        if type(baud) is not int or baud < 1:
            raise Skip("baud is not an integer of at least 1")
        start = event["t_ps"]
        data = event["bytes"]
        for index, bit in enumerate(serial_bits(data)):
            new = mark if bit else space
            if new != level:
                yield start + (index * span) // baud, level, new
                level = new
        end = start + (10 * len(data) * span) // baud
        if level != mark:
            yield end, level, mark
            level = mark


def jumps(wave):
    """Each jump as (time, old level, new level), ascending."""
    if wave["kind"] == "serial":
        yield from serial_jumps(wave)
        return
    low = float(wave["initial"]["value"])
    if wave["kind"] == "step":
        yield wave["at_ps"], low, float(wave["final"]["value"])
        return
    if wave["kind"] == "steps":
        for event in wave["events"]:
            new = float(event["level"]["value"])
            yield event["t_ps"], low, new
            low = new
        return
    high = float(wave["pulsed"]["value"])
    period = wave["period_ps"]
    width = wave["width_ps"]
    # A period that does not pass the width never moves the next rise forward.
    if period <= width or period <= 0:
        raise Skip("a pulse period does not exceed its width")
    rise = wave["delay_ps"]
    while True:
        yield rise, low, high
        yield rise + width, high, low
        rise += period


def store_on_engine_grid(point_count, chips):
    """A chip already linearizes in the deck. A short run keeps ngspice's points."""
    return not chips and point_count > POINT_CAP


def on_engine_grid(netlist, times, values, gate):
    """`t_s`, `values` and `gate` at the engine's rows.

    Each row reads ngspice's points in picoseconds the way the Rust comparison
    does, so a row scores the same error as against ngspice's own points. A held
    capacitor's columns skip ngspice's t = 0 point, as the comparison does.
    """
    tran = netlist["analysis"]["transient"]
    stop = tran["stop_ps"]
    step = tran["grid_ps"]
    if step <= 0 or stop % step != 0:
        raise SystemExit(f"grid_ps {step} does not divide stop_ps {stop}")
    at_ps = [t * 1e12 for t in times]
    rows = [float(k * step) for k in range(stop // step + 1)]
    held = held_columns(netlist)

    def resample(key, series):
        if len(series) != len(times):
            raise SystemExit(f"{key} has {len(series)} points, time has {len(times)}")
        skip = int(key in held)
        return [series[0]] * skip + [
            interpolate_series(row, at_ps[skip:], series[skip:]) for row in rows[skip:]
        ]

    return (
        [row / 1e12 for row in rows],
        {key: resample(key, series) for key, series in values.items()},
        {key: resample(key, series) for key, series in gate.items()},
    )


def dense_spans(netlist, values, gate):
    """Each scored column's maximum minus its minimum over ngspice's own points.

    That is every value, each part's `p` as the Rust comparison derives it, and
    each nmos gate current under `<ref>.ig`.
    """
    series = dict(values)
    for part in netlist["parts"]:
        if part["kind"] not in NO_QUANTITIES:
            series[f"#{part['name']}.p"] = transient_power(part, values, gate)
    series.update({f"{ref}.ig": current for ref, current in gate.items()})
    held = held_columns(netlist)
    spans = {}
    for key, column in series.items():
        column = column[int(key in held) :]
        spans[key] = max(column) - min(column)
    return spans


def transient_power(part, values, gate):
    """The part's power at each point, by the Rust comparison's formula."""
    ref = f"#{part['name']}"
    if part["kind"] in BJT_KINDS:
        c, b, e = (values[f"#net.{net}"] for net in pin_nets(part))
        ic, ib = values[f"{ref}.ic"], values[f"{ref}.ib"]
        return [(c[k] - e[k]) * ic[k] + (b[k] - e[k]) * ib[k] for k in range(len(ic))]
    if part["kind"] == "nmos":
        d, g, s = (values[f"#net.{net}"] for net in pin_nets(part))
        i_d, ig = values[f"{ref}.id"], gate[ref]
        return [(d[k] - s[k]) * i_d[k] + (g[k] - s[k]) * ig[k] for k in range(len(i_d))]
    return [v * i for v, i in zip(values[f"{ref}.v"], values[f"{ref}.i"])]


def held_columns(netlist):
    """The `i` and `p` of each capacitor with an `initial` voltage."""
    return {
        f"#{part['name']}.{quantity}"
        for part in netlist["parts"]
        if part["kind"] == "capacitor" and part["initial"] is not None
        for quantity in ("i", "p")
    }


def interpolate_series(at, times, values):
    """The value at `at`, held flat past either end, by the Rust comparison's formula."""
    lo, hi = 0, len(times)
    while lo < hi:
        mid = (lo + hi) // 2
        if times[mid] <= at:
            lo = mid + 1
        else:
            hi = mid
    if lo == 0:
        return values[0]
    if lo == len(times):
        return values[-1]
    t0, t1 = times[lo - 1], times[lo]
    v0, v1 = values[lo - 1], values[lo]
    return v0 + (v1 - v0) * (at - t0) / (t1 - t0)


def op_fallback_lines(log):
    """Lines that say the operating point left the plain Newton iteration."""
    found = []
    for raw in log.splitlines():
        line = raw.strip()
        if line and any(phrase in line for phrase in OP_FALLBACKS):
            found.append(line)
    return found


def run_ngspice(deck, raw_names, label):
    """Each raw file's vectors, in the order named."""
    with tempfile.TemporaryDirectory() as tmp:
        Path(tmp, "deck.cir").write_text(deck)
        done = subprocess.run(
            ["ngspice", "-b", "deck.cir"], cwd=tmp, capture_output=True, text=True
        )
        log = f"{done.stdout}{done.stderr}"
        fallback = op_fallback_lines(log)
        if fallback:
            raise SystemExit(
                f"{label}: ngspice operating point did not converge: {'; '.join(fallback)}"
            )
        raws = [Path(tmp, name) for name in raw_names]
        if done.returncode != 0 or not all(raw.is_file() for raw in raws):
            raise SystemExit(f"ngspice failed ({done.returncode}):\n{log}")
        return [parse_raw(raw.read_text()) for raw in raws]


def parse_raw(text):
    """Vector name to its values, one per point, from an ASCII raw file."""
    lines = text.splitlines()
    start = lines.index("Variables:") + 1
    end = lines.index("Values:")
    names = [line.split("\t")[2] for line in lines[start:end]]
    tokens = " ".join(lines[end + 1 :]).split()
    width = len(names) + 1
    points = len(tokens) // width
    indices = tokens[::width]
    if not points or len(tokens) != points * width or indices != [str(k) for k in range(points)]:
        raise SystemExit(f"unexpected raw values: {tokens[: 2 * width]}")
    return {
        name: [float(token) for token in tokens[1 + j :: width]] for j, name in enumerate(names)
    }


def write_atomic(path, text):
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.")
    with os.fdopen(fd, "w") as out:
        out.write(text)
    os.replace(tmp, path)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
