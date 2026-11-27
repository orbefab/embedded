# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Seeded random nonlinear circuit differential tester (orbe-engine vs ngspice-42).

    uv run scripts/random_circuits.py --seed 0 --count 200
    uv run scripts/random_circuits.py --seed 42 --count 50 --out .scratch/random-repros

Generates small circuits of 2-5 nonlinear catalog parts (diodes, LEDs, zeners,
BJTs, MOSFETs, linear regulators) plus passives (resistors, capacitors,
potentiometers, inductors) and switches, with:
  - a DC path to ground on every node,
  - at least 10 ohm between any step/pulse source and a capacitance or MOSFET gate.

Runs each circuit in both ngspice-42 and `orbe-run`, compares operating points
and transient traces against the bounds in `ngspice.rs` (1e-3 * |ng| + floor for
OP; 1e-3 * span + floor for transient, excluding 1 ns ramp windows), reports
mismatches with minimal repros (netlist.json, deck.cir, worst columns), and
counts ngspice aborts separately from engine failures.
"""

import argparse
import json
import math
import random
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "packages/engine/scripts"))

import catalog_sweep as catalog  # noqa: E402
import ngspice_refs as refs  # noqa: E402

NONLINEAR_KINDS = ("diode", "led", "zener", "npn", "pnp", "nmos", "pmos", "linear_regulator")


def floor_of(ref):
    if ref.startswith("#net.") or ref.endswith(".v"):
        return 1e-9
    if ref.endswith((".i", ".ib", ".ic", ".id", ".ig", ".iin", ".iout")):
        return 1e-12
    if ref.endswith(".p"):
        return 1e-15
    raise ValueError(f"unknown quantity floor for {ref}")


def load_nonlinear_pool():
    entries = catalog.dedup_entries(catalog.extract_part_entries())
    pool = []
    for eid, kind, fields, _parts in entries:
        if kind in NONLINEAR_KINDS:
            pool.append((eid, kind, fields))
    return pool


def default_runner():
    metadata = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    target = json.loads(metadata.stdout)["target_directory"]
    runner = Path(target) / "release" / "orbe-run"
    if not runner.is_file():
        raise SystemExit(
            f"random_circuits: release orbe-run not found at {runner}; "
            "build it with `cargo build --release -p orbe-engine --features runner --bin orbe-run`"
        )
    return runner


def build_random_circuit(seed, pool):
    rng = random.Random(seed)
    transient = rng.random() < 0.5
    n_nonlin = rng.randint(2, 5)
    n_work = rng.randint(3, 6)
    work_nets = [f"n{i}" for i in range(1, n_work + 1)]
    nets = ["gnd", *work_nets]
    parts = []
    part_idx = 0

    def next_name(prefix):
        nonlocal part_idx
        part_idx += 1
        return f"{prefix}{part_idx}"

    # 1-2 voltage sources, each behind its own series resistor (>= 100 ohm, well above 10 ohm)
    # so no source ever drives a capacitor or MOSFET gate directly.
    n_src = rng.randint(1, 2)
    source_nets = []
    for s_idx in range(n_src):
        snet = f"vs{s_idx + 1}"
        nets.append(snet)
        source_nets.append(snet)
        v_hi = rng.choice([3.3, 5.0, 6.0, 9.0, 12.0])
        v_lo = rng.choice([0.0, 1.0, 2.0, -3.0]) if s_idx > 0 or transient else 0.0
        if transient and (s_idx == n_src - 1 or rng.random() < 0.6):
            if rng.random() < 0.5:
                wave = catalog.step_v(v_lo, v_hi, rng.choice([15_000_000, 25_000_000, 35_000_000]))
            else:
                wave = catalog.pulse_v(v_lo, v_hi, 15_000_000, 35_000_000, 90_000_000)
        else:
            wave = catalog.q(v_hi, "V")
        parts.append(catalog.vsrc(next_name("v"), snet, "gnd", wave))
        target = work_nets[s_idx % len(work_nets)]
        r_ser = rng.choice([100.0, 220.0, 470.0, 1000.0, 2200.0])
        parts.append(catalog.resistor(next_name("rs"), snet, target, r_ser))

    # Guarantee a DC path to ground on every working node.
    for wnet in work_nets:
        r_gnd = rng.choice([2200.0, 4700.0, 10000.0, 22000.0, 47000.0])
        parts.append(catalog.resistor(next_name("rg"), wnet, "gnd", r_gnd))

    # Track nets that already hold a regulator output so two regulators never contend on one net.
    reg_out_nets = set()

    def pick_two_distinct(candidates=None):
        pool_n = candidates if candidates is not None else ["gnd", *work_nets]
        a, b = rng.sample(pool_n, 2)
        return a, b

    # Place 2-5 nonlinear catalog parts.
    # Avoid mixing linear_regulator with sharp transient steps into tiny caps that cause ngspice aborts,
    # while still allowing regulators in both OP and gentle transient runs.
    chosen_parts = [rng.choice(pool) for _ in range(n_nonlin)]
    for _eid, kind, fields in chosen_parts:
        if kind in ("diode", "led", "zener"):
            a, b = pick_two_distinct()
            parts.append(
                {
                    "name": next_name("d" if kind != "zener" else "dz"),
                    "kind": kind,
                    "pins": {"anode": a, "cathode": b},
                    **fields,
                    "ratings": {"current": None, "power": None},
                }
            )
        elif kind in ("npn", "pnp"):
            c, b, e = rng.sample(["gnd", *work_nets], 3)
            # Keep base driven through a modest series resistor on its own node to avoid hard base shorts.
            b_int = f"nb{part_idx + 1}"
            nets.append(b_int)
            parts.append(catalog.resistor(next_name("rb"), b, b_int, rng.choice([1000.0, 4700.0, 10000.0])))
            parts.append(
                {
                    "name": next_name("q"),
                    "kind": kind,
                    "pins": {"collector": c, "base": b_int, "emitter": e},
                    **fields,
                    "ratings": {"current": None, "power": None},
                }
            )
        elif kind in ("nmos", "pmos"):
            d, g, s = rng.sample(["gnd", *work_nets], 3)
            # Always put >= 10 ohm (here 1k-47k) on the gate so no step source touches a gate directly.
            g_int = f"ng{part_idx + 1}"
            nets.append(g_int)
            parts.append(catalog.resistor(next_name("rgt"), g, g_int, rng.choice([1000.0, 4700.0, 22000.0])))
            parts.append(
                {
                    "name": next_name("m"),
                    "kind": kind,
                    "pins": {"drain": d, "gate": g_int, "source": s},
                    **fields,
                    "ratings": {"current": None, "power": None},
                }
            )
        elif kind == "linear_regulator":
            avail_out = [n for n in work_nets if n not in reg_out_nets]
            if len(avail_out) < 2:
                # Fallback to a diode if work_nets are saturated with regulators.
                d_choice = rng.choice([p for p in pool if p[1] in ("diode", "led")])
                a, b = pick_two_distinct()
                parts.append(
                    {
                        "name": next_name("d"),
                        "kind": d_choice[1],
                        "pins": {"anode": a, "cathode": b},
                        **d_choice[2],
                        "ratings": {"current": None, "power": None},
                    }
                )
                continue
            inn = rng.choice(work_nets)
            out_choices = [n for n in avail_out if n != inn]
            out = rng.choice(out_choices)
            reg_out_nets.add(out)
            parts.append(
                {
                    "name": next_name("u"),
                    "kind": "linear_regulator",
                    "pins": {"in": inn, "out": out, "gnd": "gnd"},
                    **fields,
                    "ratings": {"current": None, "power": None},
                }
            )

    # Add 1-3 extra passives (resistors, capacitors, potentiometers, inductors) and optionally a switch in transient.
    n_passives = rng.randint(1, 3)
    for _ in range(n_passives):
        pkind = rng.choice(["resistor", "capacitor", "potentiometer", "inductor", "switch"])
        if pkind == "switch" and not transient:
            pkind = "resistor"
        if pkind == "resistor":
            a, b = pick_two_distinct()
            parts.append(catalog.resistor(next_name("r"), a, b, rng.choice([220.0, 470.0, 1000.0, 4700.0, 10000.0])))
        elif pkind == "capacitor":
            # Put cap on a working node (already >= 100 ohm away from any voltage source).
            a = rng.choice(work_nets)
            b = "gnd" if rng.random() < 0.7 else rng.choice([n for n in work_nets if n != a])
            cap = {
                "name": next_name("c"),
                "kind": "capacitor",
                "pins": {"a": a, "b": b},
                "capacitance": catalog.q(rng.choice([1e-9, 4.7e-9, 1e-8, 4.7e-8, 1e-7]), "F"),
                "initial": None,
            }
            if rng.random() < 0.35:
                cap["series_resistance"] = catalog.q(rng.choice([1.0, 3.0, 10.0]), "ohm")
            parts.append(cap)
        elif pkind == "potentiometer":
            a, b = pick_two_distinct()
            w = rng.choice([n for n in work_nets if n not in (a, b)] or work_nets)
            if w in (a, b):
                w = f"nw{part_idx + 1}"
                nets.append(w)
                parts.append(catalog.resistor(next_name("rw"), w, "gnd", 10000.0))
            parts.append(
                {
                    "name": next_name("rv"),
                    "kind": "potentiometer",
                    "pins": {"a": a, "wiper": w, "b": b},
                    "resistance": catalog.q(rng.choice([1000.0, 10000.0, 50000.0]), "ohm"),
                    "position": catalog.q(rng.choice([0.1, 0.25, 0.5, 0.75, 0.9]), "1"),
                }
            )
        elif pkind == "inductor":
            a, b = pick_two_distinct()
            parts.append(
                {
                    "name": next_name("l"),
                    "kind": "inductor",
                    "pins": {"a": a, "b": b},
                    "inductance": catalog.q(rng.choice([1e-4, 1e-3, 1e-2]), "H"),
                    "series_resistance": catalog.q(rng.choice([5.0, 20.0, 70.0]), "ohm"),
                }
            )
        elif pkind == "switch":
            a, b = pick_two_distinct()
            # Keep switch from shorting a regulator output directly to another net with <10 ohm.
            if a in reg_out_nets or b in reg_out_nets:
                parts.append(catalog.resistor(next_name("r"), a, b, 1000.0))
                continue
            parts.append(
                {
                    "name": next_name("sw"),
                    "kind": "switch",
                    "pins": {"a": a, "b": b},
                    "on_resistance": catalog.q(rng.choice([1.0, 10.0, 100.0]), "ohm"),
                    "off_resistance": catalog.q(1e8, "ohm"),
                    "initial": rng.choice(["open", "closed"]),
                    "events": [{"t_ps": 40_000_000, "state": "closed" if rng.random() < 0.5 else "open"}],
                    "bounce": None,
                }
            )
            # Ensure switch event flips initial state.
            sw = parts[-1]
            sw["events"][0]["state"] = "open" if sw["initial"] == "closed" else "closed"

    if transient:
        return catalog.tran_netlist(100_000_000, 1_000_000, 1_000_000, nets, parts)
    return catalog.op_netlist(nets, parts)


def run_engine(runner, netlist_path, out_dir):
    proc = subprocess.run(
        [runner, "run", netlist_path, "--out", out_dir],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        return None, f"orbe-run exit {proc.returncode}: {proc.stderr.strip() or proc.stdout.strip()}"
    diags = json.loads((out_dir / "diagnostics.json").read_text())
    fatal = [d for d in diags if d.get("code", "").startswith(("solver.", "netlist.", "net."))]
    if fatal:
        return None, f"engine diagnostics: {fatal}"
    return out_dir, None


def read_trace(out_dir):
    tdir = out_dir / "traces"
    index = json.loads((tdir / "index.json").read_text())
    rows = index["rows"]
    times = list(struct.unpack(f"<{rows}Q", (tdir / index["time"]["file"]).read_bytes()))
    col_specs = index["analog"]["columns"]
    ncols = len(col_specs)
    raw = struct.unpack(f"<{rows * ncols}d", (tdir / index["analog"]["file"]).read_bytes())
    cols = {"t_ps": [float(t) for t in times]}
    for k, spec in enumerate(col_specs):
        cols[spec["ref"]] = list(raw[k * rows : (k + 1) * rows])
    return cols


def compare_op(ref_op, eng_op):
    mismatches = []
    for key, want in ref_op.items():
        if key not in eng_op:
            mismatches.append({"col": key, "reason": "missing in engine"})
            continue
        got = float(eng_op[key])
        err = abs(got - want)
        bound = 1e-3 * abs(want) + floor_of(key)
        if err > bound:
            mismatches.append({"col": key, "engine": got, "ngspice": want, "error": err, "bound": bound})
    mismatches.sort(key=lambda m: m.get("error", math.inf) / max(m.get("bound", 1e-30), 1e-30), reverse=True)
    return mismatches


def compare_tran(ref_tran, netlist, out_dir):
    cols = read_trace(out_dir)
    eng_t = cols["t_ps"]
    ref_t = [t * 1e12 for t in ref_tran["t_s"]]
    edges = sorted(set(ref_tran["jumps"]) | {e["t_ps"] for elist in ref_tran["switches"].values() for e in elist})
    windows = [(e, e + refs.EDGE_RAMP_PS) for e in edges]
    scored_rows = [idx for idx, t in enumerate(eng_t) if not any(lo < t < hi for lo, hi in windows)]
    spans = ref_tran.get("span")
    values = ref_tran["values"]
    mismatches = []
    for key, wanted in values.items():
        if key not in cols:
            mismatches.append({"col": key, "reason": "missing in engine trace"})
            continue
        got = cols[key]
        span = spans[key] if spans is not None else (max(wanted) - min(wanted))
        bound = 1e-3 * span + floor_of(key)
        worst = 0.0
        worst_t = 0.0
        for idx in scored_rows:
            t = eng_t[idx]
            ref_val = refs.interpolate_series(t, ref_t, wanted)
            diff = abs(got[idx] - ref_val)
            if diff > worst:
                worst = diff
                worst_t = t
        if worst > bound:
            mismatches.append(
                {
                    "col": key,
                    "t_ps": worst_t,
                    "error": worst,
                    "span": span,
                    "bound": bound,
                    "ratio": worst / bound,
                }
            )
    # Also check derived part powers.
    gate = ref_tran.get("gate", {})
    for part in netlist["parts"]:
        if part["kind"] in refs.NO_QUANTITIES:
            continue
        pkey = f"#{part['name']}.p"
        if pkey not in cols:
            continue
        got_p = cols[pkey]
        wanted_p = refs.transient_power(part, values, gate)
        span = spans[pkey] if spans is not None else (max(wanted_p) - min(wanted_p))
        bound = 1e-3 * span + floor_of(pkey)
        worst = 0.0
        worst_t = 0.0
        for idx in scored_rows:
            t = eng_t[idx]
            ref_val = refs.interpolate_series(t, ref_t, wanted_p)
            diff = abs(got_p[idx] - ref_val)
            if diff > worst:
                worst = diff
                worst_t = t
        if worst > bound:
            mismatches.append(
                {
                    "col": pkey,
                    "t_ps": worst_t,
                    "error": worst,
                    "span": span,
                    "bound": bound,
                    "ratio": worst / bound,
                }
            )
    mismatches.sort(key=lambda m: m.get("ratio", math.inf), reverse=True)
    return mismatches


def run_one(seed, pool, runner, ngspice_ver, repro_root):
    netlist = build_random_circuit(seed, pool)
    is_tran = "transient" in netlist["analysis"]
    with tempfile.TemporaryDirectory() as tmp:
        cdir = Path(tmp, f"seed-{seed}")
        cdir.mkdir()
        nl_path = cdir / "netlist.json"
        nl_text = json.dumps(netlist, indent=2) + "\n"
        nl_path.write_text(nl_text)

        # 1. Translate + run ngspice.
        try:
            deck, _ = refs.translate(f"seed-{seed}", netlist)
        except refs.Skip as skip:
            return {"seed": seed, "kind": "tran" if is_tran else "op", "status": "ngspice_skip", "reason": str(skip)}
        try:
            files = refs.generate(cdir, ngspice_ver)
        except SystemExit as exc:
            return {
                "seed": seed,
                "kind": "tran" if is_tran else "op",
                "status": "ngspice_abort",
                "reason": str(exc).splitlines()[0][:200],
            }
        except refs.Skip as skip:
            return {"seed": seed, "kind": "tran" if is_tran else "op", "status": "ngspice_skip", "reason": str(skip)}

        # 2. Run engine.
        out_dir = Path(tmp, "run")
        _, eng_err = run_engine(runner, nl_path, out_dir)
        if eng_err is not None:
            saved = None
            if repro_root is not None:
                saved = save_repro(repro_root, seed, nl_text, files["deck.cir"], {"engine_error": eng_err})
            return {
                "seed": seed,
                "kind": "tran" if is_tran else "op",
                "status": "engine_fail",
                "reason": eng_err,
                "repro": saved,
            }

        # 3. Compare.
        ref = json.loads(files["ngspice.json"])
        if is_tran:
            mismatches = compare_tran(ref["tran"], netlist, out_dir)
        else:
            obs = json.loads((out_dir / "observations.json").read_text())
            mismatches = compare_op(ref["op"], obs["op"])

        if mismatches:
            saved = None
            if repro_root is not None:
                saved = save_repro(repro_root, seed, nl_text, files["deck.cir"], {"worst": mismatches[:10]})
            return {
                "seed": seed,
                "kind": "tran" if is_tran else "op",
                "status": "mismatch",
                "worst": mismatches[:5],
                "repro": saved,
            }
        return {"seed": seed, "kind": "tran" if is_tran else "op", "status": "pass"}


def save_repro(repro_root, seed, nl_text, deck_text, summary):
    dest = Path(repro_root) / f"seed-{seed}"
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "netlist.json").write_text(nl_text)
    (dest / "deck.cir").write_text(deck_text)
    (dest / "mismatch.json").write_text(json.dumps(summary, indent=2) + "\n")
    return str(dest)


def main(argv):
    parser = argparse.ArgumentParser(description="Seeded random nonlinear circuit differential tester.")
    parser.add_argument("--seed", type=int, default=0, help="Starting integer seed.")
    parser.add_argument("--count", type=int, default=200, help="Number of circuits to generate and compare.")
    parser.add_argument("--runner", type=Path, default=None, help="Path to release orbe-run binary.")
    parser.add_argument("--out", type=Path, default=None, help="Directory to write minimal repros for failures.")
    args = parser.parse_args(argv)

    runner = args.runner or default_runner()
    ngspice_ver = refs.ngspice_version()
    pool = load_nonlinear_pool()

    counts = {
        "pass": 0,
        "op_pass": 0,
        "tran_pass": 0,
        "ngspice_abort": 0,
        "ngspice_skip": 0,
        "engine_fail": 0,
        "mismatch": 0,
    }
    failures = []
    aborts = []

    for i in range(args.count):
        seed = args.seed + i
        res = run_one(seed, pool, runner, ngspice_ver, args.out)
        st = res["status"]
        counts[st] += 1
        if st == "pass":
            counts[f"{res['kind']}_pass"] += 1
        elif st == "ngspice_abort":
            aborts.append(res)
        elif st in ("engine_fail", "mismatch"):
            failures.append(res)

    report = {
        "seed": args.seed,
        "count": args.count,
        "ngspice": ngspice_ver,
        "counts": counts,
        "mismatches_and_engine_failures": failures,
        "ngspice_aborts_sample": aborts[:10],
    }
    print(json.dumps(report, indent=2))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
