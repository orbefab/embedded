#!/usr/bin/env bash
# Builds `orbe-run` natively and for wasm32-wasip1, runs both on every example in
# packages/engine/tests/circuits with the blob cache as --blobs, and requires the same folder from
# each, file for file, after masking manifest.json's created_at. When that run exits non-zero, it
# prints the run's diagnostics. It needs bash 4.2 or later, rustup with the wasm32-wasip1 target,
# and node with node:wasi. It installs nothing: a missing target exits 2 with the command that adds
# it, and a node without node:wasi prints a skip line and passes.
#
# Usage: packages/engine/scripts/wasi-check.sh [SCRATCH_DIR]
#
# SCRATCH_DIR defaults to .scratch/wasi in the repo, and the script removes it when it passes.
set -euo pipefail

if [[ $# -gt 1 ]]; then
  echo "usage: $0 [SCRATCH_DIR]" >&2
  exit 2
fi
for tool in node rustup; do
  if ! command -v "$tool" >/dev/null; then
    echo "$tool not found" >&2
    exit 2
  fi
done
if ! node --disable-warning=ExperimentalWarning --input-type=module -e 'import { WASI } from "node:wasi"' >/dev/null 2>&1; then
  echo "wasi-check: skip WASI orbe-run, node:wasi is not available"
  exit 0
fi
if ! rustup target list --installed | grep -qx wasm32-wasip1; then
  echo "wasi-check: rustup has no wasm32-wasip1 target. Add it with:" >&2
  echo "  rustup target add wasm32-wasip1" >&2
  exit 2
fi
repo=$(cd "$(dirname "$0")/../../.." && pwd)
scratch=${1:-$repo/.scratch/wasi}
mkdir -p "$scratch"
scratch=$(cd "$scratch" && pwd)
rm -rf "$scratch/native" "$scratch/wasi"

cd "$repo"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
cargo build -p orbe-circuit --features runner --bin orbe-run --release --locked
cargo build -p orbe-circuit --features runner --bin orbe-run --release --locked --target wasm32-wasip1
target=$(cargo metadata --format-version 1 --no-deps |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
runner=$target/release/orbe-run
wasm=$target/wasm32-wasip1/release/orbe-run.wasm
# The blobs no example commits, which every side reads from the cache.
ORBE_BLOBS=$("$repo/packages/engine/scripts/blobs.sh")
export ORBE_BLOBS
names=()
for netlist in packages/engine/tests/circuits/*/netlist.json; do
  name=$(basename "$(dirname "$netlist")")
  names+=("$name")
  "$runner" run "$netlist" --out "$scratch/native/$name" --blobs "$ORBE_BLOBS" >/dev/null
done
mkdir -p "$scratch/wasi"
node --disable-warning=ExperimentalWarning --input-type=module - "$wasm" "$repo" "$scratch/native" "$scratch/wasi" "$scratch/wasi-report" "$ORBE_BLOBS" "${names[@]}" <<'JS'
import { WASI } from 'node:wasi';
import { closeSync, openSync } from 'node:fs';
import { readFile, readdir, stat } from 'node:fs/promises';
import { join } from 'node:path';

const [wasmPath, repo, nativeRoot, wasiRoot, reportPath, blobs, ...names] = process.argv.slice(2);
const mod = await WebAssembly.compile(await readFile(wasmPath));

async function files(dir, prefix = '') {
  const found = await readdir(dir);
  found.sort();
  const out = [];
  for (const name of found) {
    const rel = prefix ? `${prefix}/${name}` : name;
    const path = join(dir, name);
    if ((await stat(path)).isDirectory()) out.push(...(await files(path, rel)));
    else out.push(rel);
  }
  return out;
}

function maskCreatedAt(text) {
  const doc = JSON.parse(text);
  delete doc.created_at;
  return JSON.stringify(doc);
}

for (const name of names) {
  const reportFd = openSync(reportPath, 'w');
  const wasi = new WASI({
    version: 'preview1',
    args: ['orbe-run', 'run', `/repo/packages/engine/tests/circuits/${name}/netlist.json`, '--out', `/out/${name}`, '--blobs', '/blobs'],
    preopens: { '/repo': repo, '/out': wasiRoot, '/blobs': blobs },
    stdout: reportFd,
    returnOnExit: true,
  });
  let code;
  try {
    const inst = await WebAssembly.instantiate(mod, wasi.getImportObject());
    code = wasi.start(inst);
  } catch (error) {
    closeSync(reportFd);
    console.error(`wasi-check: WASI orbe-run packages/engine/tests/circuits/${name} failed: ${error.message ?? error}`);
    process.exit(1);
  }
  closeSync(reportFd);
  if (code !== 0) {
    const report = (await readFile(reportPath, 'utf8')).trim();
    console.error(`wasi-check: WASI orbe-run packages/engine/tests/circuits/${name} exited ${code ?? 'null'}`);
    let why = report;
    try {
      const doc = JSON.parse(report);
      if (Array.isArray(doc.diagnostics) && doc.diagnostics.length > 0) {
        why = doc.diagnostics.map((item) => item.message ?? JSON.stringify(item)).join('\n');
      }
    } catch {
      why = report;
    }
    if (why) console.error(why);
    process.exit(code || 1);
  }
  const nativeDir = join(nativeRoot, name);
  const wasiDir = join(wasiRoot, name);
  let nativeFiles;
  let wasiFiles;
  try {
    nativeFiles = await files(nativeDir);
    wasiFiles = await files(wasiDir);
  } catch (error) {
    console.error(`wasi-check: packages/engine/tests/circuits/${name} folder missing: ${error.message ?? error}`);
    process.exit(1);
  }
  const nativeSet = new Set(nativeFiles);
  const wasiSet = new Set(wasiFiles);
  const onlyNative = nativeFiles.filter((rel) => !wasiSet.has(rel));
  const onlyWasi = wasiFiles.filter((rel) => !nativeSet.has(rel));
  if (onlyNative.length || onlyWasi.length) {
    console.error(
      `wasi-check: packages/engine/tests/circuits/${name} file list differs, native only ${onlyNative}, wasi only ${onlyWasi}`,
    );
    process.exit(1);
  }
  for (const rel of nativeFiles) {
    const nativeBytes = await readFile(join(nativeDir, rel));
    const wasiBytes = await readFile(join(wasiDir, rel));
    if (rel === 'manifest.json') {
      const nativeText = maskCreatedAt(nativeBytes.toString('utf8'));
      const wasiText = maskCreatedAt(wasiBytes.toString('utf8'));
      if (nativeText !== wasiText) {
        console.error(`wasi-check: packages/engine/tests/circuits/${name} manifest.json differs after masking created_at`);
        process.exit(1);
      }
      continue;
    }
    if (!nativeBytes.equals(wasiBytes)) {
      console.error(`wasi-check: packages/engine/tests/circuits/${name} ${rel} differs`);
      process.exit(1);
    }
  }
}

console.log(`wasi-check: WASI orbe-run matches native on ${names.length} examples`);
JS
rm -rf "$scratch"
