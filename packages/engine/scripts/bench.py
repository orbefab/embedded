#!/usr/bin/env python3
"""Build and run the native, WASI, and Worker benchmarks."""

import hashlib
import importlib.metadata
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CHROME_ROOT = Path.home() / ".cache/ms-playwright"
MIN_FREE = 20 * 1024**3
SAME = ("landings", "stops", "end_ps", "simulated_ps")
FRAME_PS = 16_666_666_667
SERIAL_EXAMPLE = "atmega-analog-serial"


def require_disk(path: Path) -> None:
    probe = path if path.exists() else path.parent
    free = shutil.disk_usage(probe).free
    if free < MIN_FREE:
        gib = free / 1024**3
        sys.exit(f"stopping: {gib:.1f} GiB free on {probe}, under 20 GiB")


def scratch_bench() -> Path:
    path = ROOT / ".scratch" / "bench"
    path.mkdir(parents=True, exist_ok=True)
    return path


def choose_target() -> Path:
    if os.environ.get("CARGO_TARGET_DIR"):
        path = Path(os.environ["CARGO_TARGET_DIR"]).resolve()
    else:
        digest = hashlib.sha256(str(ROOT.resolve()).encode()).hexdigest()[:12]
        path = scratch_bench() / f"target-{digest}"
    path.mkdir(parents=True, exist_ok=True)
    return path


def cargo_env(target: Path) -> dict[str, str]:
    env = os.environ.copy()
    env["CARGO_BUILD_JOBS"] = "4"
    env["CARGO_TARGET_DIR"] = str(target)
    return env


def run(cmd: list[str], target: Path, **kwargs) -> subprocess.CompletedProcess:
    require_disk(target)
    print("+", " ".join(cmd), flush=True)
    return subprocess.run(cmd, cwd=ROOT, env=cargo_env(target), check=True, **kwargs)


def chrome() -> Path:
    matches = sorted(CHROME_ROOT.glob("**/chrome-headless-shell"))
    matches += sorted(CHROME_ROOT.glob("**/headless_shell"))
    if not matches:
        sys.exit(f"no Chromium under {CHROME_ROOT}")
    return matches[-1]


def cpu_model() -> str:
    for line in Path("/proc/cpuinfo").read_text().splitlines():
        if line.startswith("model name"):
            return line.split(":", 1)[1].strip()
    return "unknown"


def read_load() -> list[float]:
    parts = Path("/proc/loadavg").read_text().split()
    if len(parts) < 3:
        sys.exit("/proc/loadavg has fewer than three numbers")
    return [float(part) for part in parts[:3]]


def load_text(sample: list[float]) -> str:
    return f"{sample[0]:.2f} {sample[1]:.2f} {sample[2]:.2f}"


def load_sentence(
    native: list[float],
    wasi: list[float],
    worker: list[float],
    after: list[float],
) -> str:
    return (
        "Load before native was "
        f"{load_text(native)}, before WASI was {load_text(wasi)}, "
        f"before the Worker was {load_text(worker)}, "
        f"and after the last target was {load_text(after)}."
    )


def cpu_governor() -> str:
    path = Path("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
    if path.exists():
        return path.read_text().strip()
    return "unknown"


def text_version(cmd: list[str]) -> str:
    out = subprocess.run(cmd, check=True, capture_output=True, text=True, cwd=ROOT)
    return out.stdout.strip() or out.stderr.strip()


def ensure_wasm_targets() -> None:
    listed = subprocess.run(
        ["rustup", "target", "list", "--installed"],
        check=True,
        capture_output=True,
        text=True,
        cwd=ROOT,
    )
    installed = set(listed.stdout.split())
    for name in ("wasm32-wasip1", "wasm32-unknown-unknown"):
        if name not in installed:
            subprocess.run(
                ["rustup", "target", "add", name],
                check=True,
                cwd=ROOT,
            )


def serve(directory: Path) -> tuple[ThreadingHTTPServer, int]:
    class Handler(SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(directory), **kwargs)

        def end_headers(self) -> None:
            self.send_header("Cross-Origin-Opener-Policy", "same-origin")
            self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
            self.send_header("Cross-Origin-Resource-Policy", "same-origin")
            super().end_headers()

        def log_message(self, fmt: str, *args) -> None:
            return

    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    sock.close()
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, port


def stage_web(stage: Path, wasm: Path, names: list[str]) -> None:
    if stage.exists():
        shutil.rmtree(stage)
    (stage / "examples").mkdir(parents=True)
    shutil.copy(ROOT / "packages/engine/bench/web/index.html", stage / "index.html")
    shutil.copy(ROOT / "packages/engine/bench/web/worker.js", stage / "worker.js")
    shutil.copy(wasm, stage / "orbe_engine_bench.wasm")
    (stage / "names.json").write_text(json.dumps(names))
    for name in names:
        dest = stage / "examples" / name
        dest.mkdir()
        shutil.copy(ROOT / "packages/engine/tests/circuits" / name / "netlist.json", dest / "netlist.json")


def run_worker(stage: Path, runs: int, out: Path) -> tuple[str, str, bool]:
    from playwright.sync_api import sync_playwright

    query = f"runs={runs}"
    server, port = serve(stage)
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(
                executable_path=str(chrome()),
                headless=True,
            )
            version = browser.version
            page = browser.new_page()
            page.goto(f"http://127.0.0.1:{port}/index.html?{query}", timeout=60_000)
            isolated = page.evaluate(
                "() => window.__bench && Object.hasOwn(window.__bench, 'crossOriginIsolated')"
                " ? window.__bench.crossOriginIsolated : undefined"
            )
            if isolated is None:
                sys.exit("worker crossOriginIsolated is missing")
            if isolated is not True:
                sys.exit("worker crossOriginIsolated is false")
            print(f"crossOriginIsolated {isolated}", flush=True)
            page.evaluate("() => window.__bench.start()")
            page.wait_for_function(
                "() => window.__bench && window.__bench.done",
                timeout=60 * 60 * 1000,
            )
            payload = page.evaluate("() => window.__bench")
            browser.close()
    finally:
        server.shutdown()
    if payload.get("error"):
        sys.exit(payload["error"])
    out.write_text(json.dumps(payload["report"]))
    return version, importlib.metadata.version("playwright"), isolated


def seconds(ns: int) -> str:
    return f"{ns / 1e9:.3f}"


def times_real(simulated_ps: int, wall_ns: int) -> str:
    if wall_ns == 0:
        return "inf"
    return f"{simulated_ps / wall_ns * 1e-3:.2f}"


def spread(block: dict) -> str:
    return f"{seconds(block['median'])} ({seconds(block['min'])} to {seconds(block['max'])})"


def core_cell(row: dict) -> str:
    bare = row.get("bare_core_ns")
    if not bare:
        return "none"
    return seconds(bare["median"])


def index_rows(report: dict) -> dict[tuple[str, str], dict]:
    found = {}
    for row in report["workloads"]:
        key = (row["example"], row["variant"])
        if key in found:
            sys.exit(f"{key[0]} {key[1]} repeated in one report")
        found[key] = row
    return found


def require_same(native: dict, wasi: dict, worker: dict) -> None:
    maps = {
        "native": index_rows(native),
        "wasi": index_rows(wasi),
        "worker": index_rows(worker),
    }
    keys = set(maps["native"]) | set(maps["wasi"]) | set(maps["worker"])
    for example, variant in sorted(keys):
        present = {label: maps[label].get((example, variant)) for label in maps}
        for label, row in present.items():
            if row is None:
                sys.exit(f"{example} {variant} missing on {label}")
        for field in SAME:
            values = {label: present[label][field] for label in maps}
            if len(set(values.values())) > 1:
                detail = ", ".join(f"{label}={values[label]}" for label in maps)
                sys.exit(f"{example} {variant} {field} differs: {detail}")


def wall_per_sim(wall_ns: int, simulated_ps: int) -> float:
    return wall_ns / simulated_ps * 1e3


def combined_table(native: dict, wasi: dict, worker: dict) -> str:
    lines = [
        "| example | variant | simulated | native | WASI | Worker | native x | WASI x | Worker x | native core | WASI core | Worker core | landings | stops |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    notes = []
    reports = (("native", native), ("WASI", wasi), ("Worker", worker))
    wasi_rows = index_rows(wasi)
    worker_rows = index_rows(worker)
    for row in native["workloads"]:
        example, variant = row["example"], row["variant"]
        others = {
            "wasi": wasi_rows[(example, variant)],
            "worker": worker_rows[(example, variant)],
        }
        lines.append(
            "| {example} | {variant} | {sim:.3f} | {native} | {wasi} | {worker} | {nx} | {wx} | {kx} | {nc} | {wc} | {kc} | {landings} | {stops} |".format(
                example=example,
                variant=variant,
                sim=row["simulated_ps"] / 1e12,
                native=spread(row["wall_ns"]),
                wasi=spread(others["wasi"]["wall_ns"]),
                worker=spread(others["worker"]["wall_ns"]),
                nx=times_real(row["simulated_ps"], row["wall_ns"]["median"]),
                wx=times_real(row["simulated_ps"], others["wasi"]["wall_ns"]["median"]),
                kx=times_real(row["simulated_ps"], others["worker"]["wall_ns"]["median"]),
                nc=core_cell(row),
                wc=core_cell(others["wasi"]),
                kc=core_cell(others["worker"]),
                landings=row["landings"],
                stops=row["stops"],
            )
        )
    for label, report in reports:
        found = 0
        for row in report["workloads"]:
            if row.get("secs") is None:
                continue
            found += 1
            block = row["wall_ns"]
            med = wall_per_sim(block["median"], row["simulated_ps"])
            lo = wall_per_sim(block["min"], row["simulated_ps"])
            hi = wall_per_sim(block["max"], row["simulated_ps"])
            notes.append(
                f"{row['example']} {row['variant']} takes {med:.3f} wall seconds per simulated second on {label} ({lo:.3f} to {hi:.3f})."
            )
        if found == 0:
            sys.exit(f"{label} report has no row with secs")
    return "\n".join(lines + notes)


def parse_args(argv: list[str]) -> tuple[int, Path]:
    runs = 3
    out = None
    i = 0
    while i < len(argv):
        arg = argv[i]
        if arg == "--runs":
            if i + 1 >= len(argv):
                sys.exit("--runs needs a value")
            try:
                runs = int(argv[i + 1])
            except ValueError:
                sys.exit("--runs needs a positive integer")
            if runs < 1:
                sys.exit("--runs needs a positive integer")
            i += 2
            continue
        if arg.startswith("-"):
            sys.exit(f"unknown argument {arg}")
        if out is not None:
            sys.exit(f"unexpected argument {arg}")
        out = Path(arg).resolve()
        i += 1
    if out is None:
        out = (ROOT / ".scratch" / "bench" / "report.json").resolve()
    return runs, out


def main() -> None:
    runs, out = parse_args(sys.argv[1:])
    out.parent.mkdir(parents=True, exist_ok=True)
    target = choose_target()
    require_disk(target)
    scratch = Path(tempfile.mkdtemp(prefix="bench-", dir=scratch_bench()))
    try:
        ensure_wasm_targets()
        run(["cargo", "build", "-p", "orbe-engine-bench", "--release"], target)
        run(
            [
                "cargo",
                "build",
                "-p",
                "orbe-engine-bench",
                "--release",
                "--target",
                "wasm32-wasip1",
                "--bin",
                "orbe-engine-bench",
            ],
            target,
        )
        run(
            [
                "cargo",
                "rustc",
                "-p",
                "orbe-engine-bench",
                "--release",
                "--target",
                "wasm32-unknown-unknown",
                "--lib",
                "--crate-type",
                "cdylib",
            ],
            target,
        )
        native_bin = target / "release" / "orbe-engine-bench"
        wasi_wasm = target / "wasm32-wasip1" / "release" / "orbe-engine-bench.wasm"
        worker_wasm = target / "wasm32-unknown-unknown" / "release" / "orbe_engine_bench.wasm"
        names = json.loads(
            subprocess.run(
                [str(native_bin), "--list"],
                check=True,
                capture_output=True,
                text=True,
                cwd=ROOT,
            ).stdout
        )
        stage = scratch / "stage"
        stage_web(stage, worker_wasm, names)
        native_out = scratch / "native.json"
        wasi_out = scratch / "wasi.json"
        worker_out = scratch / "worker.json"
        load_native = read_load()
        run(
            [
                str(native_bin),
                "--runs",
                str(runs),
                "--examples",
                str(ROOT / "packages/engine/tests/circuits"),
                "--out",
                str(native_out),
            ],
            target,
        )
        wasi_dir = scratch / "wasi-out"
        wasi_dir.mkdir(parents=True, exist_ok=True)
        node = scratch / "wasi.mjs"
        node.write_text(
            """
import { WASI } from 'node:wasi';
import { readFile } from 'node:fs/promises';
const [wasm, examples, out, runs] = process.argv.slice(2);
const wasi = new WASI({
  version: 'preview1',
  args: ['orbe-engine-bench', '--runs', runs, '--examples', '/examples', '--out', '/out/wasi.json'],
  preopens: { '/examples': examples, '/out': out },
  returnOnExit: true,
});
const mod = await WebAssembly.compile(await readFile(wasm));
const inst = await WebAssembly.instantiate(mod, wasi.getImportObject());
const code = wasi.start(inst);
if (code !== 0) process.exit(code);
"""
        )
        load_wasi = read_load()
        run(
            [
                "node",
                "--disable-warning=ExperimentalWarning",
                str(node),
                str(wasi_wasm),
                str(ROOT / "packages/engine/tests/circuits"),
                str(wasi_dir),
                str(runs),
            ],
            target,
        )
        shutil.copy(wasi_dir / "wasi.json", wasi_out)
        load_worker = read_load()
        browser_version, playwright_version, isolated = run_worker(stage, runs, worker_out)
        load_after = read_load()
        native = json.loads(native_out.read_text())
        wasi = json.loads(wasi_out.read_text())
        worker = json.loads(worker_out.read_text())
        for label, report in ("native", native), ("wasi", wasi), ("worker", worker):
            if report["target"] != label:
                sys.exit(f"{label} report is labeled {report['target']}")
            for row in report["workloads"]:
                if row["errors"]:
                    sys.exit(f"{label} {row['example']} {row['variant']} reported errors")
        require_same(native, wasi, worker)
        recorded_load = load_sentence(
            load_native, load_wasi, load_worker, load_after
        )
        machine = {
            "cpu": cpu_model(),
            "cores": os.cpu_count(),
            "governor": cpu_governor(),
            "load": {
                "native": load_native,
                "wasi": load_wasi,
                "worker": load_worker,
                "after": load_after,
            },
            "rustc": text_version(["rustc", "--version"]),
            "cargo": text_version(["cargo", "--version"]),
            "node": text_version(["node", "--version"]),
            "browser": browser_version,
            "playwright": playwright_version,
            "cross_origin_isolated": isolated,
        }
        print(
            f"cpu {machine['cpu']}, cores {machine['cores']}, governor {machine['governor']}"
        )
        print(recorded_load)
        print(f"rustc {machine['rustc']}")
        print(f"cargo {machine['cargo']}")
        print(f"node {machine['node']}")
        print(f"browser {machine['browser']}")
        print(f"playwright {machine['playwright']}")
        table = combined_table(native, wasi, worker)
        print(table)
        report = {
            "machine": machine,
            "runs": runs,
            "native": native,
            "wasi": wasi,
            "worker": worker,
            "table": table,
        }
        out.write_text(json.dumps(report, indent=2) + "\n")
        print(out)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    main()
