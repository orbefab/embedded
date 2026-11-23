#!/usr/bin/env python3
"""The published orbe reads every part and Block, or asks for a newer orbe.

A part is checked in a small scene that sets its required params. A Block is
`orbe check` of its scene. A clean read passes. A refusal passes only when
every diagnostic says the format is newer than that binary reads.
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
NEWER = "is newer than this binary reads"


def strip_jsonc(text: str) -> str:
    out: list[str] = []
    i = 0
    n = len(text)
    in_str = False
    esc = False
    while i < n:
        c = text[i]
        if in_str:
            out.append(c)
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            out.append(c)
            i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            i += 2
            while i < n and text[i] not in "\n\r":
                i += 1
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            i += 2
            while i + 1 < n and not (text[i] == "*" and text[i + 1] == "/"):
                i += 1
            i = min(i + 2, n)
            continue
        out.append(c)
        i += 1
    cleaned = "".join(out)
    return re.sub(r",(\s*[}\]])", r"\1", cleaned)


def load(path: Path) -> dict:
    return json.loads(strip_jsonc(path.read_text()))


def owns(decl: dict) -> bool:
    return any(key in decl for key in ("file", "unit", "choices"))


def passed_to(part: dict, name: str) -> tuple[str, str] | None:
    specs = part.get("parts") or {}
    if not isinstance(specs, dict):
        return None
    for spec in specs.values():
        if not isinstance(spec, dict):
            continue
        ty = spec.get("type")
        if not isinstance(ty, str) or ty.startswith(".") or ty.startswith("@"):
            continue
        for key, value in spec.items():
            if isinstance(value, dict) and value.get("param") == name:
                return ty, key
    return None


def owner_of(parts: dict[str, dict], part_id: str, name: str) -> tuple[str, str, dict]:
    current = part_id
    for _ in range(16):
        part = parts.get(current)
        if part is None:
            break
        decl = (part.get("params") or {}).get(name)
        if not isinstance(decl, dict) or owns(decl):
            return current, name, decl if isinstance(decl, dict) else {}
        found = passed_to(part, name)
        if not found:
            return current, name, decl
        current, name = found
    return part_id, name, {}


def required_params(parts: dict[str, dict], part_id: str) -> list[tuple[str, object]]:
    """Params the scene must set, with the example the part file gives."""
    out: list[tuple[str, object]] = []
    for name, decl in (parts[part_id].get("params") or {}).items():
        if not isinstance(decl, dict):
            continue
        explicit = "required" in decl and decl.get("required") is not False
        if not explicit and "default" in decl:
            continue
        owner, oname, odecl = owner_of(parts, part_id, name)
        if not explicit:
            if owner == part_id and oname == name and not owns(odecl) and "required" not in odecl:
                continue
            if not ("required" in odecl and odecl.get("required") is not False):
                continue
            shown = odecl
        else:
            shown = decl
            if "example" not in decl and "example" in odecl:
                shown = odecl
        example = shown.get("example")
        if example is None:
            continue
        if "file" in odecl and isinstance(example, str):
            example = f"parts/{owner}/{example}"
        out.append((name, example))
    return out


def scene_for(parts: dict[str, dict], part_id: str) -> dict:
    inst: dict[str, object] = {"type": part_id}
    for name, example in required_params(parts, part_id):
        inst[name] = example
    placed: dict[str, object] = {"u1": inst}
    body: dict[str, object] = {"format": 1, "parts": placed}
    ground = parts[part_id].get("ground")
    if not isinstance(ground, str):
        placed["g"] = {"type": "./probe-ground.jsonc"}
        body["ground"] = "g.neg"
    return body


PROBE_GROUND = {
    "format": 1,
    "pins": {"pos": [1], "neg": [2]},
    "model": {
        "kind": "voltage_source",
        "voltage": {"value": 0, "unit": "V", "source": "user"},
        "pins": {"p": "pos", "n": "neg"},
    },
}


def run_check(orbe: Path, scene: Path, cwd: Path) -> list[dict]:
    proc = subprocess.run(
        [str(orbe), "check", str(scene), "--json"],
        cwd=cwd,
        text=True,
        capture_output=True,
        check=False,
    )
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError:
        raw = (proc.stdout + proc.stderr).strip()
        return [{"code": "unparsed", "message": raw[:2000]}]
    if data.get("ok"):
        return []
    diags = data.get("data", {}).get("diagnostics")
    if diags:
        return diags
    error = data.get("error") or {}
    return [{"code": error.get("code"), "message": error.get("message") or proc.stderr.strip()}]


def accepted(diags: list[dict]) -> bool:
    return not diags or all(NEWER in (d.get("message") or "") for d in diags)


def show(item: str, diags: list[dict]) -> None:
    print(f"{item}: the published orbe refuses this", file=sys.stderr)
    for diag in diags:
        file = str(diag.get("file") or "")
        place = ""
        if diag.get("line"):
            place = f":{diag.get('line')}:{diag.get('column')}"
        print(f"  {diag.get('code')} {file}{place} {diag.get('message')}", file=sys.stderr)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def folder_hash(files: dict[str, bytes]) -> str:
    lines = [f"{sha256(body)}  {path}\n" for path, body in files.items()]
    lines.sort(key=lambda line: line[66:])
    return "sha256:" + sha256("".join(lines).encode())


def write_record(root: Path, name: str, part: bytes, orbe: str) -> dict:
    digest = sha256(part)
    (root / "files").mkdir(exist_ok=True)
    (root / "files" / digest).write_bytes(part)
    record = {
        "$schema": "item.schema.json",
        "type": "part",
        "format": 1,
        "name": name,
        "version": "1.0.0",
        "title": name,
        "license": "MIT",
        "registryDependencies": [],
        "files": [
            {
                "path": "part.jsonc",
                "url": f"files/{digest}",
                "sha256": digest,
                "bytes": len(part),
                "license": "MIT",
            }
        ],
        "hash": folder_hash({"part.jsonc": part}),
        "real": {},
        "pins": [],
        "params": [],
        "ratings": [],
        "notSimulated": [],
    }
    raw = json.dumps(record, separators=(",", ":")).encode()
    (root / f"{name}@1.0.0.json").write_bytes(raw)
    return {
        "name": name,
        "type": "part",
        "version": "1.0.0",
        "title": name,
        "url": f"{name}@1.0.0.json",
        "sha256": sha256(raw),
        "bytes": len(raw),
        "orbe": orbe,
    }


def released_add(orbe: Path) -> int:
    """orbe 0.2.0 installs a part it can read, and is told to update for one it cannot."""
    with tempfile.TemporaryDirectory(prefix="orbe-v020-") as raw:
        root = Path(raw) / "registry"
        root.mkdir()
        rows = [
            write_record(root, "lamp", b'{"format":1,"part":"lamp","pins":["a","b"]}\n', "0.2.0"),
            write_record(root, "glow", b'{"format":4,"part":"glow","pins":["a","b"]}\n', "0.2.1"),
        ]
        rows.sort(key=lambda row: row["name"])
        index = {"$schema": "index.schema.json", "format": 1, "name": "@acme", "items": rows}
        (root / "index.json").write_text(json.dumps(index, separators=(",", ":")))
        env = os.environ.copy()
        env["ORBE_REGISTRY"] = f"file://{root}/{{name}}.json"
        project = Path(raw) / "project"
        project.mkdir()

        def invoke(args: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
            return subprocess.run(
                [str(orbe), *args],
                cwd=cwd,
                env=env,
                text=True,
                capture_output=True,
                check=False,
            )

        added = invoke(["add", "lamp"], project)
        lamp = project / "parts" / "lamp" / "part.jsonc"
        if added.returncode != 0 or not lamp.is_file():
            print(added.stdout + added.stderr, file=sys.stderr)
            print("orbe 0.2.0 did not install lamp", file=sys.stderr)
            return 1
        glow = invoke(["add", "glow"], project)
        told = glow.stdout + glow.stderr
        if glow.returncode != 0:
            if NEWER not in told and "needs a newer orbe" not in told:
                print(told, file=sys.stderr)
                print("orbe 0.2.0 refused glow without asking for a newer orbe", file=sys.stderr)
                return 1
        else:
            scene = project / "scene.jsonc"
            scene.write_text('{"format":1,"parts":{"g":{"type":"glow"}}}\n')
            checked = invoke(["check", str(scene)], project)
            told = checked.stdout + checked.stderr
            if NEWER not in told and "needs a newer orbe" not in told:
                print(told, file=sys.stderr)
                print("orbe 0.2.0 installed glow and did not ask for a newer orbe", file=sys.stderr)
                return 1
        print("orbe 0.2.0 installed lamp and asked for a newer orbe for glow")
        return 0


def main() -> int:
    if len(sys.argv) == 3 and sys.argv[1] == "--released-add":
        return released_add(Path(sys.argv[2]).resolve())
    if len(sys.argv) != 2:
        print("usage: scripts/check_client.py <orbe>", file=sys.stderr)
        return 2
    orbe = Path(sys.argv[1]).resolve()
    version = subprocess.run([str(orbe), "--version"], text=True, capture_output=True, check=False)
    print((version.stdout or version.stderr).strip())

    parts: dict[str, dict] = {}
    for folder in sorted(p for p in (ROOT / "parts").iterdir() if (p / "part.jsonc").is_file()):
        parts[folder.name] = load(folder / "part.jsonc")

    failed = 0
    with tempfile.TemporaryDirectory(prefix="orbe-client-") as raw:
        work = Path(raw)
        os.symlink(ROOT / "parts", work / "parts")
        (work / "probe-ground.jsonc").write_text(json.dumps(PROBE_GROUND))
        for part_id in parts:
            scene = work / "scene.jsonc"
            scene.write_text(json.dumps(scene_for(parts, part_id)))
            diags = run_check(orbe, scene, work)
            if not accepted(diags):
                show(part_id, diags)
                failed += 1

    for scene in sorted((ROOT / "blocks").glob("*/scene.jsonc")):
        diags = run_check(orbe, scene, ROOT)
        if not accepted(diags):
            show(scene.parent.name, diags)
            failed += 1

    if failed:
        print(f"{failed} items the published orbe refuses for another reason", file=sys.stderr)
        return 1
    print(f"{len(parts)} parts and the Blocks pass")
    return 0


if __name__ == "__main__":
    sys.exit(main())
