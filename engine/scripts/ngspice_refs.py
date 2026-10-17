# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Write each example's ngspice reference from its netlist.json.

    uv run scripts/ngspice_refs.py [--check] [examples/<name> ...]
    uv run scripts/ngspice_refs.py --self-test

For every example it translates netlist.json into deck.cir, runs `ngspice -b`
on it, and writes the operating point to ngspice.json. With no example
arguments it takes every examples/*/netlist.json. `--check` regenerates in
memory and exits 1 if any committed byte differs, or if a skipped example
already has ngspice.json. stdout is one JSON document.

A part with no kind, an unknown kind, a part with no pins, or a missing pin
raises Skip. `--self-test` checks that without ngspice.
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
            "part": {"name": "c1", "kind": "capacitor", "pins": {"a": "a", "b": "gnd"}},
            "commit": False,
            "exit": 0,
            "reason": "kind capacitor has no translation",
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
            "part": {"name": "c1", "kind": "capacitor", "pins": {"a": "a", "b": "gnd"}},
            "commit": True,
            "exit": 1,
            "reason": "kind capacitor has no translation",
            "failure": "committed ngspice.json was not checked",
        },
    ]
    cases = []
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for spec in specs:
            example = root / spec["name"].replace(" ", "-")
            example.mkdir()
            netlist = {"ground": "gnd", "nets": ["gnd", "a"], "parts": [spec["part"]]}
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
    document = {"ok": all(case["ok"] for case in cases), "cases": cases}
    print(json.dumps(document, indent=2))
    return 0 if document["ok"] else 1


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
    values = run_ngspice(deck)
    op = {}
    for variable, value in values.items():
        if variable not in mapping:
            raise SystemExit(f"{example.name}: unexpected ngspice vector {variable}")
        op[mapping[variable]] = value
    op[f"#net.{netlist['ground']}"] = 0.0
    for part in netlist["parts"]:
        first, second = (net_voltage(op, net) for net in pin_nets(part))
        op[f"#{part['name']}.v"] = first - second
    reference = {
        "format": FORMAT,
        "netlist_sha256": hashlib.sha256(netlist_bytes).hexdigest(),
        "deck_sha256": hashlib.sha256(deck.encode()).hexdigest(),
        "ngspice": version,
        "options": OPTIONS,
        "op": op,
    }
    text = json.dumps(reference, indent=2, sort_keys=True) + "\n"
    return {"deck.cir": deck, "ngspice.json": text}


PIN_ORDER = {
    "resistor": ("a", "b"),
    "voltage_source": ("p", "n"),
    "diode": ("anode", "cathode"),
    "led": ("anode", "cathode"),
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
    # SPICE names are case-insensitive, and ngspice reads `gnd` as ground.
    for kind, names in (("net", nets), ("part", [p["name"] for p in parts])):
        lowered = [n.lower() for n in names]
        if len(set(lowered)) != len(lowered):
            raise Skip(f"two {kind} names are equal ignoring case")
    if any(n.lower() == "gnd" and n != ground for n in nets):
        raise Skip("a net other than ground is named gnd")

    def node(net):
        return "0" if net == ground else net.lower()

    mapping = {f"v({net.lower()})": f"#net.{net}" for net in nets if net != ground}
    lines = [
        f"* {title}: generated by scripts/ngspice_refs.py from netlist.json. Do not edit.",
        f".options {OPTIONS}",
    ]
    saves = []
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
        else:
            raise Skip(f"kind {kind} has no translation")
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


def number(value):
    return repr(float(value["value"]))


def run_ngspice(deck):
    with tempfile.TemporaryDirectory() as tmp:
        Path(tmp, "deck.cir").write_text(deck)
        done = subprocess.run(
            ["ngspice", "-b", "deck.cir"], cwd=tmp, capture_output=True, text=True
        )
        raw = Path(tmp, RAW)
        if done.returncode != 0 or not raw.is_file():
            raise SystemExit(f"ngspice failed ({done.returncode}):\n{done.stdout}{done.stderr}")
        return parse_raw(raw.read_text())


def parse_raw(text):
    """Vector name to value, from an ASCII raw file with one point."""
    lines = text.splitlines()
    start = lines.index("Variables:") + 1
    end = lines.index("Values:")
    names = [line.split("\t")[2] for line in lines[start:end]]
    tokens = " ".join(lines[end + 1 :]).split()
    if tokens[0] != "0" or len(tokens) != len(names) + 1:
        raise SystemExit(f"unexpected raw values: {tokens}")
    return {name: float(token) for name, token in zip(names, tokens[1:])}


def write_atomic(path, text):
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.")
    with os.fdopen(fd, "w") as out:
        out.write(text)
    os.replace(tmp, path)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
