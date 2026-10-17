# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Write each example's ngspice reference from its netlist.json.

    uv run scripts/ngspice_refs.py [--check] [examples/<name> ...]
    uv run scripts/ngspice_refs.py --self-test

For every example it translates netlist.json into deck.cir, runs `ngspice -b`
on it, and writes the operating point, or the transient waveforms, to
ngspice.json. With no example arguments it takes every
examples/*/netlist.json. `--check` regenerates in memory and exits 1 if any
committed byte differs, or if a skipped example already has ngspice.json.
stdout is one JSON document.

A part with no kind, an unknown kind, a part with no pins, or a missing pin
raises Skip. `--self-test` checks that, and the bounce test vectors, without
ngspice.
"""

import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FORMAT = 0
OPTIONS = "temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12"
RAW = "op.raw"
TRAN_RAW = "tran.raw"
# The engine's first step after a contact edge. The Rust test pins both.
EDGE_RAMP_PS = 1000
# The engine's least time between two edges of one switch.
MIN_GAP_PS = 2000
M64 = (1 << 64) - 1


class Skip(Exception):
    """The netlist has something this translator does not write."""


def main(argv):
    if "--self-test" in argv:
        return self_test()
    check = "--check" in argv
    targets = [Path(a) for a in argv if a != "--check"]
    if not targets:
        targets = sorted(p.parent for p in (ROOT / "examples").glob("*/netlist.json"))
    report = report_examples(targets, check, ngspice_version())
    print(json.dumps(report, indent=2))
    return exit_code(report)


def report_examples(targets, check, version):
    report = {"ngspice": version, "checked" if check else "written": [], "skipped": []}
    if check:
        report["drift"] = []
    for example in targets:
        example = example.resolve()
        label = label_of(example)
        try:
            files = generate(example, version)
        except Skip as skip:
            entry = {"example": label, "reason": str(skip)}
            if check and (example / "ngspice.json").is_file():
                entry["failure"] = "committed ngspice.json was not checked"
            report["skipped"].append(entry)
            continue
        if check:
            for name, text in files.items():
                path = example / name
                if not path.is_file() or path.read_bytes() != text.encode():
                    report["drift"].append(f"{label}/{name}")
            report["checked"].append(label)
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
    return 1 if report.get("drift") or unchecked else 0


def self_test():
    """Skip paths, with no ngspice. stdout is one JSON document."""
    specs = [
        {
            "name": "unknown kind",
            "part": {"name": "l1", "kind": "inductor", "pins": {"a": "a", "b": "gnd"}},
            "commit": False,
            "exit": 0,
            "reason": "kind inductor has no translation",
            "failure": None,
        },
        {
            "name": "missing pin",
            "part": {"name": "r1", "kind": "resistor", "pins": {"a": "a"}},
            "commit": False,
            "exit": 0,
            "reason": "kind resistor is missing pin b",
            "failure": None,
        },
        {
            "name": "no kind",
            "part": {"name": "r1", "pins": {"a": "a", "b": "gnd"}},
            "commit": False,
            "exit": 0,
            "reason": "part has no kind",
            "failure": None,
        },
        {
            "name": "no pins",
            "part": {"name": "r1", "kind": "resistor"},
            "commit": False,
            "exit": 0,
            "reason": "kind resistor has no pins",
            "failure": None,
        },
        {
            "name": "list kind",
            "part": {"name": "r1", "kind": ["resistor"], "pins": {"a": "a", "b": "gnd"}},
            "commit": False,
            "exit": 0,
            "reason": "netlist.json is not a netlist the engine accepts (TypeError)",
            "failure": None,
        },
        {
            "name": "no name",
            "part": {"kind": "resistor", "pins": {"a": "a", "b": "gnd"}},
            "commit": False,
            "exit": 0,
            "reason": "netlist.json is not a netlist the engine accepts (KeyError)",
            "failure": None,
        },
        {
            "name": "undeclared net",
            "part": {"name": "r1", "kind": "resistor", "pins": {"a": "a", "b": "ghost"}},
            "commit": False,
            "exit": 0,
            "reason": "part r1 pins a net that is not declared",
            "failure": None,
        },
        {
            "name": "committed reference",
            "part": {"name": "l1", "kind": "inductor", "pins": {"a": "a", "b": "gnd"}},
            "commit": True,
            "exit": 1,
            "reason": "kind inductor has no translation",
            "failure": "committed ngspice.json was not checked",
        },
        {
            "name": "switch at the operating point",
            "part": {"name": "sw1", "kind": "switch", "pins": {"a": "a", "b": "gnd"}},
            "commit": False,
            "exit": 0,
            "reason": "a switch is translated only for a transient analysis",
            "failure": None,
        },
        {
            "name": "reserved net",
            "analysis": {"transient": {"stop_ps": 5000, "max_step_ps": 1000}},
            "nets": ["gnd", "BP1"],
            "part": {"name": "r1", "kind": "resistor", "pins": {"a": "BP1", "b": "gnd"}},
            "commit": False,
            "exit": 0,
            "reason": "a transient deck reserves names starting with ctl_ or bp",
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
            if spec["commit"]:
                (example / "ngspice.json").write_text("{}\n")
            report = report_examples([example], True, "ngspice-42")
            code = exit_code(report)
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
    cases.append(bounce_vectors())
    document = {"ok": all(case["ok"] for case in cases), "cases": cases}
    print(json.dumps(document, indent=2))
    return 0 if document["ok"] else 1


def bounce_vectors():
    """The Rust unit tests in crates/core/src/netlist/bounce.rs pin the same numbers."""
    rng = SplitMix64(0)
    first = [rng.next(), rng.next()]
    button = {
        "name": "sw1",
        "events": [
            {"t_ps": 2000000000, "state": "closed"},
            {"t_ps": 12000000000, "state": "open"},
        ],
        "bounce": {"window": {"value": 1000000000}, "max_bounces": {"value": 4}},
    }
    edges = [(edge["t_ps"], edge["state"][0]) for edge in switch_edges(7, button)]
    ok = (
        first == [0xE220A8397B1DCDAF, 0x6E789E6AA1B965F4]
        and fnv1a64(b"sw1") == 0x826ADE195D0A12BA
        and bounce_stream(7, "sw1", 0).state == 0xD65F52CA546DD32E
        and edges
        == [
            (2000000000, "c"),
            (2040015938, "o"),
            (2114234716, "c"),
            (2665597584, "o"),
            (2854176182, "c"),
            (12000000000, "o"),
            (12089763725, "c"),
            (12337840633, "o"),
            (12576192418, "c"),
            (12607405695, "o"),
            (12637715418, "c"),
            (12691361990, "o"),
            (12720009883, "c"),
            (12727823423, "o"),
        ]
    )
    return {"name": "bounce vectors", "ok": ok}


def mix(z):
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
    return z ^ (z >> 31)


class SplitMix64:
    def __init__(self, state):
        self.state = state & M64

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & M64
        return mix(self.state)

    def below(self, n):
        """In [0, n): the high 64 bits of the 128-bit product."""
        return (self.next() * n) >> 64


def fnv1a64(data):
    h = 0xCBF29CE484222325
    for byte in data:
        h = ((h ^ byte) * 0x100000001B3) & M64
    return h


def bounce_stream(seed, name, index):
    return SplitMix64(mix(mix(seed ^ fnv1a64(name.encode())) ^ index))


def switch_edges(seed, part):
    """Every contact edge, nominal and bounce, ascending, as the engine's `bounce::expand`."""
    edges = []
    for index, event in enumerate(part["events"]):
        edges.append({"t_ps": event["t_ps"], "state": event["state"]})
        bounce = part["bounce"]
        if bounce is None:
            continue
        rng = bounce_stream(seed, part["name"], index)
        flips = 2 * (1 + rng.below(bounce["max_bounces"]["value"]))
        slack = bounce["window"]["value"] - flips * MIN_GAP_PS
        draws = sorted(rng.below(slack + 1) for _ in range(flips))
        state = event["state"]
        for j, draw in enumerate(draws, start=1):
            state = "open" if state == "closed" else "closed"
            edges.append({"t_ps": event["t_ps"] + draw + j * MIN_GAP_PS, "state": state})
    return edges


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
    vectors = run_ngspice(deck, TRAN_RAW if transient else RAW)
    values = {}
    for variable, column in vectors.items():
        if transient and variable == "time":
            continue
        if variable not in mapping:
            raise SystemExit(f"{example.name}: unexpected ngspice vector {variable}")
        values[mapping[variable]] = column
    if transient:
        count = len(vectors["time"])
        values[f"#net.{netlist['ground']}"] = [0.0] * count
        part_voltages(example.name, netlist, values, count)
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
            "switches": switches(netlist),
            "t_s": vectors["time"],
            "values": values,
        }
    else:
        reference["op"] = op_point(netlist, values)
    text = json.dumps(reference, indent=2, sort_keys=True) + "\n"
    return {"deck.cir": deck, "ngspice.json": text}


def op_point(netlist, values):
    op = {ref: column[0] for ref, column in values.items()}
    op[f"#net.{netlist['ground']}"] = 0.0
    for part in netlist["parts"]:
        first, second = (net_voltage(op, net) for net in pin_nets(part))
        op[f"#{part['name']}.v"] = first - second
    return op


def part_voltages(label, netlist, values, count):
    """Each part's `.v` on every transient row, from its pin-order node voltages."""
    for part in netlist["parts"]:
        try:
            first, second = (values[f"#net.{net}"] for net in pin_nets(part))
        except KeyError as missing:
            raise SystemExit(f"{label}: no node voltage {missing}") from None
        if len(first) != count or len(second) != count:
            raise SystemExit(f"{label}: {part['name']} voltage has the wrong length")
        values[f"#{part['name']}.v"] = [a - b for a, b in zip(first, second)]


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
    "voltage_source": ("p", "n"),
    "diode": ("anode", "cathode"),
    "led": ("anode", "cathode"),
    "capacitor": ("a", "b"),
    "switch": ("a", "b"),
}


def pin_nets(part):
    if "kind" not in part:
        raise Skip("part has no kind")
    kind = part["kind"]
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
    if transient and any(n.lower().startswith(("ctl_", "bp")) for n in names):
        raise Skip("a transient deck reserves names starting with ctl_ or bp")

    def node(net):
        return "0" if net == ground else net.lower()

    mapping = {f"v({net.lower()})": f"#net.{net}" for net in nets if net != ground}
    lines = [
        f"* {title}: generated by scripts/ngspice_refs.py from netlist.json. Do not edit.",
        f".options {OPTIONS}",
    ]
    saves = []
    corners = set()
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
        elif kind == "voltage_source":
            lines.append(f"v{name} {pins} dc {number(part['voltage'])}")
            mapping[f"i(v{name})"] = ref
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
        elif kind == "capacitor":
            lines.append(f"c{name} {pins} {number(part['capacitance'])}")
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
        else:
            raise Skip(f"kind {kind} has no translation")
    if not transient:
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
    stop = transient["stop_ps"]
    voltages = [f"v({net.lower()})" for net in nets if net != ground]
    sources = [f"i(v{p['name'].lower()})" for p in parts if p["kind"] == "voltage_source"]
    lines += [
        ".control",
        " ".join(["save", *voltages, *sources, *saves]),
        # ngspice picks its own points under this cap, which does not depend on the engine's max_step.
        f"tran {stop // 1000}p {stop}p 0 {stop // 1000}p",
        "set filetype=ascii",
        f"write {TRAN_RAW}",
        ".endc",
        ".end",
    ]
    return "\n".join(lines) + "\n", mapping


def number(value):
    return repr(float(value["value"]))


def run_ngspice(deck, raw_name):
    with tempfile.TemporaryDirectory() as tmp:
        Path(tmp, "deck.cir").write_text(deck)
        done = subprocess.run(
            ["ngspice", "-b", "deck.cir"], cwd=tmp, capture_output=True, text=True
        )
        raw = Path(tmp, raw_name)
        if done.returncode != 0 or not raw.is_file():
            raise SystemExit(f"ngspice failed ({done.returncode}):\n{done.stdout}{done.stderr}")
        return parse_raw(raw.read_text())


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
