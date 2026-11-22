#!/usr/bin/env node
// The asset step: checks a part's body model and writes the merged GLB and asset.json.
// scripts/model.sh builds a part from model.py and runs this step. See docs/asset-step.md.

import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { Logger, NodeIO } from '@gltf-transform/core';
import { parse } from 'jsonc-parser';
import { partNodes } from './geometry.mjs';
import { CheckFailed, anchorCheck, composition, frame, merge, names, seating } from './checks.mjs';

const quiet = new Logger(Logger.Verbosity.WARN);
const io = new NodeIO().setLogger(quiet);

const USAGE = `usage: asset-step.mjs --raw <dir> [--parts <dir>] [<id> ...]

--raw reads <raw>/<id>/<id>.glb and <id>.anchors.json, the files model.py just wrote, and
writes the merged GLB, the anchors and asset.json into the part folder after the checks pass.
With no ids, every part that has model.py. It does not read the GLB already in the part folder.`;

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function digest(bytes) {
  return { sha256: sha256(bytes), bytes: bytes.length };
}

function readJsonc(path) {
  const errors = [];
  const tree = parse(readFileSync(path, 'utf8'), errors, { allowTrailingComma: true });
  if (errors.length) throw new Error(`${path} is not valid JSONC`);
  return tree;
}

function readPart(partsDir, id) {
  const tree = readJsonc(join(partsDir, id, 'part.jsonc'));
  const leads = new Set();
  for (const value of Object.values(tree.pins ?? {})) {
    if (Array.isArray(value)) value.filter(Number.isInteger).forEach((lead) => leads.add(lead));
  }
  const view = tree.view ?? {};
  return {
    id,
    leads,
    viewNodes: [
      ...(view.glow ?? []).map((e) => e.node),
      ...(view.tint ?? []).map((e) => e.node),
      ...(view.recolour ?? []).flatMap((e) => e.nodes),
    ],
    jointNodes: Object.values(tree.joints ?? {}).map((j) => j.node),
    children: new Map(Object.entries(tree.parts ?? {}).map(([role, child]) => [role, child.type])),
    layout: new Map(Object.entries(tree.layout ?? {})),
  };
}

/** The merged GLB and anchors already in the part folder, for a child of a composed part. */
function folderModel(partsDir, id) {
  const glbPath = join(partsDir, id, `${id}.glb`);
  if (!existsSync(glbPath)) return null;
  const glb = readFileSync(glbPath);
  const anchorsPath = join(partsDir, id, `${id}.anchors.json`);
  const anchors = existsSync(anchorsPath) ? readFileSync(anchorsPath) : null;
  return { glb, anchors };
}

/** The raw export model.py wrote under `rawDir`, plus the hash of model.py. */
function rawSource(rawDir, partsDir, id) {
  const glbPath = join(rawDir, id, `${id}.glb`);
  const scriptPath = join(partsDir, id, 'model.py');
  if (!existsSync(glbPath) || !existsSync(scriptPath)) return null;
  const glb = readFileSync(glbPath);
  const anchorsPath = join(rawDir, id, `${id}.anchors.json`);
  const anchors = existsSync(anchorsPath) ? readFileSync(anchorsPath) : null;
  const script = readFileSync(scriptPath);
  return { script: 'model.py', sha256: sha256(script), bytes: script.length, glb, anchors };
}

function noAnchors(id) {
  return { part: id, units: 'mm', anchors: [] };
}

function readAnchors(src, id) {
  return src.anchors ? JSON.parse(src.anchors.toString('utf8')) : noAnchors(id);
}

function readAsset(partsDir, id) {
  const path = join(partsDir, id, 'asset.json');
  return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')) : null;
}

/** The model of every part below `part`. Each child is the merged GLB in its folder. */
async function childModels(partsDir, part, found = new Map()) {
  for (const type of part.children.values()) {
    if (found.has(type)) continue;
    const asset = readAsset(partsDir, type);
    if (!asset) continue;
    const src = folderModel(partsDir, type);
    if (!src) throw new CheckFailed('composition', `child type \`${type}\` has no GLB in its folder`);
    const child = readPart(partsDir, type);
    const nodes = partNodes((await io.readBinary(src.glb)).setLogger(quiet));
    found.set(type, { part: child, nodes, anchors: readAnchors(src, type), drawCalls: asset.draw_calls });
    await childModels(partsDir, child, found);
  }
  return found;
}

async function step(part, src, partsDir) {
  const anchors = readAnchors(src, part.id);
  const doc = (await io.readBinary(src.glb)).setLogger(quiet);
  const checks = { frame: frame(doc) };
  const nodes = partNodes(doc);
  checks.anchors = anchorCheck(part, anchors, nodes);
  checks.seating = seating(anchors, nodes);
  checks.names = names(part, nodes);
  const merged = await merge(doc, part, anchors);
  frame(doc);
  checks.merge = merged.detail;
  if (part.children.size) {
    const model = { part, nodes, anchors, drawCalls: merged.drawCalls };
    const composed = composition(model, await childModels(partsDir, part));
    checks.composition = { ...composed.detail, draw_calls: composed.drawCalls };
  }
  const glb = stampFactors(Buffer.from(await io.writeBinary(doc)), doc);
  const files = { [`${part.id}.glb`]: digest(glb) };
  if (src.anchors) files[`${part.id}.anchors.json`] = digest(src.anchors);
  const asset = {
    part: part.id,
    source: { script: src.script, sha256: src.sha256, bytes: src.bytes },
    files,
    box: merged.box,
    kept_nodes: merged.kept,
    draw_calls: merged.drawCalls,
    checks,
  };
  return { glb, asset };
}

/** Put metallicFactor and roughnessFactor on every material that declares one.

gltf-transform omits a factor that equals glTF's default (metallic 1, roughness 1).
A metal lead would then lose metallicFactor and a loader could not tell a declared
1 from a factor the writer never wrote. Synthetic fixtures that leave both at the
default are unchanged. */
function stampFactors(bytes, doc) {
  const specs = new Map(
    doc.getRoot().listMaterials().map((material) => [
      material.getName(),
      {
        metallic: material.getMetallicFactor(),
        roughness: material.getRoughnessFactor(),
        alpha: material.getBaseColorFactor()[3],
      },
    ]),
  );
  const declared = [...specs.values()].some((spec) => spec.metallic !== 1 || spec.roughness !== 1 || spec.alpha < 0.999);
  if (!declared) return bytes;
  const jsonLength = bytes.readUInt32LE(12);
  if (bytes.readUInt32LE(16) !== 0x4e4f534a) throw new Error('GLB JSON chunk missing');
  const json = JSON.parse(bytes.subarray(20, 20 + jsonLength).toString('utf8'));
  for (const mat of json.materials ?? []) {
    const spec = specs.get(mat.name);
    if (!spec) throw new Error(`GLB material ${mat.name} is not in the document`);
    mat.pbrMetallicRoughness ??= {};
    mat.pbrMetallicRoughness.metallicFactor = spec.metallic;
    mat.pbrMetallicRoughness.roughnessFactor = spec.roughness;
    if (spec.alpha < 0.999) {
      const color = mat.pbrMetallicRoughness.baseColorFactor ?? [1, 1, 1, 1];
      mat.pbrMetallicRoughness.baseColorFactor = color.length < 4 ? [...color, spec.alpha] : [...color.slice(0, 3), spec.alpha];
      mat.alphaMode = 'BLEND';
    }
  }
  let text = Buffer.from(JSON.stringify(json));
  const pad = (4 - (text.length % 4)) % 4;
  if (pad) text = Buffer.concat([text, Buffer.alloc(pad, 0x20)]);
  const rest = bytes.subarray(20 + jsonLength);
  const total = 12 + 8 + text.length + rest.length;
  const header = Buffer.alloc(20);
  header.writeUInt32LE(0x46546c67, 0);
  header.writeUInt32LE(2, 4);
  header.writeUInt32LE(total, 8);
  header.writeUInt32LE(text.length, 12);
  header.writeUInt32LE(0x4e4f534a, 16);
  return Buffer.concat([header, text, rest]);
}

/** JSON with two-space indents and each list of numbers or names on one line. */
function stringify(value) {
  return JSON.stringify(value, null, 2).replace(/\[\s+([^[\]{}]*?)\s+\]/g, (_, inner) => `[${inner.split(/,\s+/).join(', ')}]`);
}

/** Writes every file to a temporary name first, so a failed write leaves the old files. */
function write(partsDir, part, src, { glb, asset }) {
  const folder = join(partsDir, part.id);
  const files = [[`${part.id}.glb`, glb], ['asset.json', `${stringify(asset)}\n`]];
  if (src.anchors) files.push([`${part.id}.anchors.json`, src.anchors]);
  const paths = files.map(([name]) => join(folder, name));
  try {
    files.forEach(([, bytes], i) => writeFileSync(`${paths[i]}.tmp`, bytes));
  } catch (error) {
    paths.forEach((path) => rmSync(`${path}.tmp`, { force: true }));
    throw error;
  }
  paths.forEach((path) => renameSync(`${path}.tmp`, path));
}

/** Orders the jobs so each part comes after the parts it holds, then by id. */
function childrenFirst(jobs) {
  const byId = new Map(jobs.map((job) => [job.part.id, job]));
  const ordered = [];
  const seen = new Set();
  const visit = (job) => {
    if (seen.has(job)) return;
    seen.add(job);
    for (const type of job.part.children.values()) if (byId.has(type)) visit(byId.get(type));
    ordered.push(job);
  };
  [...byId.keys()].sort().forEach((id) => visit(byId.get(id)));
  return ordered;
}

function report(id, error) {
  if (!(error instanceof CheckFailed)) console.error(error);
  console.log(`FAIL  ${id}  ${error.check ?? 'error'}: ${error.message}`);
}

async function main() {
  const { values, positionals } = parseArgs({
    options: {
      parts: { type: 'string' },
      raw: { type: 'string' },
    },
    allowPositionals: true,
  });
  if (!values.raw) {
    console.error(USAGE);
    return 2;
  }
  const here = dirname(fileURLToPath(import.meta.url));
  const partsDir = resolve(values.parts ?? join(here, '../../parts'));
  const ids = positionals.length
    ? positionals
    : readdirSync(partsDir).filter((id) => existsSync(join(partsDir, id, 'model.py')));

  const jobs = [];
  const failedIds = new Set();
  for (const id of ids) {
    try {
      const src = rawSource(values.raw, partsDir, id);
      if (!src) throw new CheckFailed('source', `no raw export for ${id}. Run scripts/model.sh ${id}`);
      jobs.push({ part: readPart(partsDir, id), src });
    } catch (error) {
      report(id, error);
      failedIds.add(id);
    }
  }

  for (const { part, src } of childrenFirst(jobs)) {
    try {
      const blocked = [...part.children.values()].filter((type) => failedIds.has(type));
      if (blocked.length) throw new CheckFailed('composition', `children ${[...new Set(blocked)].join(', ')} failed their checks`);
      const out = await step(part, src, partsDir);
      write(partsDir, part, src, out);
      const composed = out.asset.checks.composition;
      const calls = composed ? `${out.asset.draw_calls}, composed ${composed.draw_calls}` : out.asset.draw_calls;
      console.log(
        `ok    ${part.id}  kept ${out.asset.kept_nodes.length} nodes, ${calls} draw calls, ` +
          `${out.glb.length} B, ${src.script}`,
      );
    } catch (error) {
      report(part.id, error);
      failedIds.add(part.id);
    }
  }
  if (failedIds.size) console.log(`${failedIds.size} part(s) failed; a failed part's files were not written`);
  return failedIds.size ? 1 : 0;
}

process.exitCode = await main();
