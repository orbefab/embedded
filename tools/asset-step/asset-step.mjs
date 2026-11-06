#!/usr/bin/env node
// The asset step: brings each part's body model from a commit of the models repo into
// parts/<id>/, checked and merged. See docs/asset-step.md.

import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { Logger, NodeIO } from '@gltf-transform/core';
import { parse } from 'jsonc-parser';
import { partNodes } from './geometry.mjs';
import { CheckFailed, anchorCheck, composition, frame, merge, names, seating } from './checks.mjs';

const REPO = 'https://github.com/orbefab/models';
const quiet = new Logger(Logger.Verbosity.WARN);
const io = new NodeIO().setLogger(quiet);

const USAGE = `usage: asset-step.mjs --models <models checkout> [--commit <rev>] [--parts <dir>] [<id> ...]

Reads GLB/parts/<id>.glb and <id>.anchors.json at <rev> of the models checkout, without
checking it out, and writes parts/<id>/<id>.glb, <id>.anchors.json and asset.json. With no
--commit, each part is rerun from the commit its asset.json records. With no ids, every part
in --parts (default: this repo's parts/) that has a GLB at that commit.`;

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function digest(bytes) {
  return { sha256: sha256(bytes), bytes: bytes.length };
}

function git(models, args) {
  return execFileSync('git', ['-C', models, ...args], { maxBuffer: 1 << 30, stdio: ['ignore', 'pipe', 'pipe'] });
}

function commitOf(models, rev) {
  return git(models, ['rev-parse', '--verify', `${rev}^{commit}`]).toString().trim();
}

function blob(models, commit, path) {
  try {
    return git(models, ['show', `${commit}:${path}`]);
  } catch {
    return null;
  }
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

function source(models, commit, id) {
  const glbPath = `GLB/parts/${id}.glb`;
  const anchorsPath = `GLB/parts/${id}.anchors.json`;
  const glb = blob(models, commit, glbPath);
  if (!glb) return null;
  const anchors = blob(models, commit, anchorsPath);
  const inputs = { glb: { path: glbPath, ...digest(glb) } };
  if (anchors) inputs.anchors = { path: anchorsPath, ...digest(anchors) };
  return { commit, glb, anchors, inputs };
}

function noAnchors(id) {
  return { part: id, units: 'mm', anchors: [] };
}

function readOutput(partsDir, id) {
  const folder = join(partsDir, id);
  const anchorsPath = join(folder, `${id}.anchors.json`);
  return {
    glb: readFileSync(join(folder, `${id}.glb`)),
    anchors: existsSync(anchorsPath) ? JSON.parse(readFileSync(anchorsPath, 'utf8')) : noAnchors(id),
    drawCalls: JSON.parse(readFileSync(join(folder, 'asset.json'), 'utf8')).draw_calls,
  };
}

async function step(part, src, partsDir) {
  const anchors = src.anchors ? JSON.parse(src.anchors.toString('utf8')) : noAnchors(part.id);
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
    const children = new Map();
    for (const type of new Set(part.children.values())) {
      if (!existsSync(join(partsDir, type, 'asset.json'))) continue;
      const out = readOutput(partsDir, type);
      children.set(type, { ...out, nodes: partNodes(await io.readBinary(out.glb)) });
    }
    const composed = composition(part, partNodes(doc), children);
    checks.composition = { ...composed.detail, draw_calls: composed.drawCalls };
  }
  const glb = Buffer.from(await io.writeBinary(doc));
  const files = { [`${part.id}.glb`]: digest(glb) };
  if (src.anchors) files[`${part.id}.anchors.json`] = digest(src.anchors);
  const asset = {
    part: part.id,
    source: { repo: REPO, commit: src.commit, inputs: src.inputs },
    files,
    box: merged.box,
    kept_nodes: merged.kept,
    draw_calls: merged.drawCalls,
    checks,
  };
  return { glb, asset };
}

function writeAtomic(path, bytes) {
  writeFileSync(`${path}.tmp`, bytes);
  renameSync(`${path}.tmp`, path);
}

/** JSON with two-space indents and each list of numbers or names on one line. */
function stringify(value) {
  return JSON.stringify(value, null, 2).replace(/\[\s+([^[\]{}]*?)\s+\]/g, (_, inner) => `[${inner.split(/,\s+/).join(', ')}]`);
}

function write(partsDir, part, src, { glb, asset }) {
  const folder = join(partsDir, part.id);
  writeAtomic(join(folder, `${part.id}.glb`), glb);
  if (src.anchors) writeAtomic(join(folder, `${part.id}.anchors.json`), src.anchors);
  writeAtomic(join(folder, 'asset.json'), `${stringify(asset)}\n`);
}

function recordedCommit(partsDir, id) {
  const path = join(partsDir, id, 'asset.json');
  return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')).source.commit : null;
}

async function main() {
  const { values, positionals } = parseArgs({
    options: { models: { type: 'string' }, commit: { type: 'string' }, parts: { type: 'string' } },
    allowPositionals: true,
  });
  if (!values.models) {
    console.error(USAGE);
    return 2;
  }
  const here = dirname(fileURLToPath(import.meta.url));
  const partsDir = resolve(values.parts ?? join(here, '../../parts'));
  const ids = positionals.length ? positionals : readdirSync(partsDir).filter((id) => existsSync(join(partsDir, id, 'part.jsonc')));
  const pinned = values.commit && commitOf(values.models, values.commit);

  const jobs = [];
  let failed = 0;
  for (const id of ids) {
    const rev = pinned || recordedCommit(partsDir, id);
    if (!rev) {
      console.log(`FAIL  ${id}  source: no --commit given and no asset.json records one`);
      failed++;
      continue;
    }
    const src = source(values.models, commitOf(values.models, rev), id);
    if (!src) {
      if (positionals.length) {
        console.log(`FAIL  ${id}  source: no GLB/parts/${id}.glb at ${rev}`);
        failed++;
      } else {
        console.log(`skip  ${id}  no GLB/parts/${id}.glb at ${rev}`);
      }
      continue;
    }
    jobs.push({ part: readPart(partsDir, id), src });
  }
  // A part with children is checked against its children's merged output, so it goes last.
  jobs.sort((a, b) => Math.sign(a.part.children.size) - Math.sign(b.part.children.size) || a.part.id.localeCompare(b.part.id));

  const failedIds = new Set();
  for (const { part, src } of jobs) {
    try {
      const blocked = [...part.children.values()].filter((type) => failedIds.has(type));
      if (blocked.length) throw new CheckFailed('composition', `children ${[...new Set(blocked)].join(', ')} failed their checks`);
      const out = await step(part, src, partsDir);
      write(partsDir, part, src, out);
      const composed = out.asset.checks.composition;
      const calls = composed ? `${out.asset.draw_calls}, composed ${composed.draw_calls}` : out.asset.draw_calls;
      console.log(
        `ok    ${part.id}  kept ${out.asset.kept_nodes.length} nodes, ${calls} draw calls, ` +
          `${out.glb.length} B, ${src.commit.slice(0, 7)}`,
      );
    } catch (error) {
      if (!(error instanceof CheckFailed)) throw error;
      console.log(`FAIL  ${part.id}  ${error.check}: ${error.message}`);
      failedIds.add(part.id);
      failed++;
    }
  }
  if (failed) console.log(`${failed} part(s) failed; a failed part's files were not written`);
  return failed ? 1 : 0;
}

process.exitCode = await main();
