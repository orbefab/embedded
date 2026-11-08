// Runs the asset step on crafted models in a scratch models repo. scripts/check-assets.sh runs it.

import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { Document, Logger, NodeIO } from '@gltf-transform/core';
import { GLTF_TURN } from './geometry.mjs';

const STEP = join(dirname(fileURLToPath(import.meta.url)), 'asset-step.mjs');
const quiet = new Logger(Logger.Verbosity.ERROR);
const io = new NodeIO().setLogger(quiet);
const { FORCE_COLOR, NO_COLOR, ...env } = process.env;

/** A GLB whose root carries glTF's turn, with `nodes` as `{name, points (mm), children}` under it. */
async function glb(nodes) {
  const doc = new Document().setLogger(quiet);
  const buffer = doc.createBuffer();
  const root = doc.createNode('root').setRotation(GLTF_TURN);
  doc.createScene().addChild(root);
  const material = doc.createMaterial('plastic');
  const add = (parent, spec) => {
    const node = doc.createNode(spec.name ?? '');
    if (spec.points) {
      const positions = doc
        .createAccessor()
        .setType('VEC3')
        .setArray(new Float32Array(spec.points.flat().map((v) => v / 1000)))
        .setBuffer(buffer);
      node.setMesh(doc.createMesh().addPrimitive(doc.createPrimitive().setMode(0).setAttribute('POSITION', positions).setMaterial(material)));
    }
    parent.addChild(node);
    (spec.children ?? []).forEach((child) => add(node, child));
  };
  nodes.forEach((spec) => add(root, spec));
  return Buffer.from(await io.writeBinary(doc));
}

const FACE = [
  [-0.5, -0.5, -1],
  [0.5, -0.5, -1],
  [0.5, 0.5, -1],
  [-0.5, 0.5, -1],
];

const leg = (over = {}) => ({ lead: 1, node: 'lead', also: [], at: [0, 0, -1], axis: [0, 0, -1], kind: 'leg', ...over });

/** A one-lead part whose lead ends in a face centred on (0, 0, -1), above a body at z = 1. */
function leaded(id, { nodes = [], anchor = {}, part = {} } = {}) {
  return {
    part: { format: 1, part: id, pins: { a: [1] }, ...part },
    nodes: [{ name: 'lead', points: FACE }, { name: 'body', points: [[0, 0, 1]] }, ...nodes],
    anchors: { part: id, units: 'mm', anchors: [leg(anchor)] },
  };
}

function board(id, children, layout, nodes = [{ name: 'pcb', points: [[0, 0, 1]] }]) {
  const parts = Object.fromEntries(Object.entries(children).map(([role, type]) => [role, { type }]));
  return { part: { format: 1, part: id, parts, layout }, nodes };
}

const BLOCK = { part: { format: 1, part: 'block' }, nodes: [{ name: 'body', points: [[0, 0, 0], [0, 0, 2]] }] };

function git(models, args) {
  return execFileSync('git', ['-C', models, '-c', 'user.name=test', '-c', 'user.email=test@example.invalid', ...args], {
    encoding: 'utf8',
  }).trim();
}

/** Commits each spec's GLB and anchors to the scratch models repo and returns the commit. */
async function publish(fx, specs) {
  mkdirSync(join(fx.models, 'GLB/parts'), { recursive: true });
  for (const [id, spec] of Object.entries(specs)) {
    if (!spec.nodes) continue;
    writeFileSync(join(fx.models, 'GLB/parts', `${id}.glb`), await glb(spec.nodes));
    if (spec.anchors) {
      const text = typeof spec.anchors === 'string' ? spec.anchors : JSON.stringify(spec.anchors);
      writeFileSync(join(fx.models, 'GLB/parts', `${id}.anchors.json`), text);
    }
  }
  git(fx.models, ['add', '-A']);
  git(fx.models, ['commit', '-q', '--allow-empty', '-m', 'models']);
  return git(fx.models, ['rev-parse', 'HEAD']);
}

const writePart = (fx, id, part) => writeFileSync(join(fx.parts, id, 'part.jsonc'), JSON.stringify(part));

async function fixture(t, specs) {
  const root = mkdtempSync(join(tmpdir(), 'asset-step-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const fx = { models: join(root, 'models'), parts: join(root, 'parts') };
  mkdirSync(fx.models);
  git(fx.models, ['init', '-q']);
  fx.commit = await publish(fx, specs);
  for (const [id, spec] of Object.entries(specs)) {
    mkdirSync(join(fx.parts, id), { recursive: true });
    writePart(fx, id, spec.part);
  }
  return fx;
}

function run(fx, ids, commit = fx.commit) {
  const args = [STEP, '--models', fx.models, '--parts', fx.parts, ...(commit ? ['--commit', commit] : []), ...ids];
  const { status, stdout, stderr } = spawnSync(process.execPath, args, { encoding: 'utf8', env });
  const lines = stdout.trimEnd().split('\n');
  return { status, stderr, line: (id) => lines.find((l) => l.split(/\s+/)[1] === id) };
}

const files = (fx, id) => readdirSync(join(fx.parts, id)).sort();
const asset = (fx, id) => JSON.parse(readFileSync(join(fx.parts, id, 'asset.json'), 'utf8'));

test('a two-number `at` places a child at z = 0', async (t) => {
  const fx = await fixture(t, {
    block: BLOCK,
    flipped: board('flipped', { u1: 'block' }, { u1: { at: [0, 0], rotate: [180, 0, 0] } }),
    raised: board('raised', { u1: 'block' }, { u1: { at: [0, 0, 5], rotate: [180, 0, 0] } }),
  });
  const out = run(fx, ['block', 'flipped', 'raised']);
  assert.equal(out.status, 1);
  assert.match(out.line('flipped'), /^FAIL {2}flipped {2}composition: placed by the layout, node `u1\/body` reaches z = -2\.000 mm/);
  assert.match(out.line('raised'), /^ok/);
  assert.deepEqual(asset(fx, 'raised').checks.composition, { children: 1, lowest_node: 'pcb', z_mm: 1, draw_calls: 2 });
});

test('a vertex that is not a finite number fails seating and composition', async (t) => {
  const fx = await fixture(t, {
    block: BLOCK,
    'nan-body': leaded('nan-body', { nodes: [{ name: 'shell', points: [[0, 0, NaN]] }] }),
    'nan-also': leaded('nan-also', { nodes: [{ name: 'tab', points: [[0, 0, NaN]] }], anchor: { also: ['tab'] } }),
    'short-rotate': board('short-rotate', { u1: 'block' }, { u1: { at: [0, 0, 5], rotate: [180] } }),
  });
  const out = run(fx, ['block', 'nan-body', 'nan-also', 'short-rotate']);
  assert.match(out.line('nan-body'), /seating: node `shell` has a vertex that is not a finite number/);
  assert.match(out.line('nan-also'), /seating: node `tab` has a vertex that is not a finite number/);
  assert.match(out.line('short-rotate'), /composition: placed by the layout, node `u1\/body` has a vertex that is not a finite number/);
});

test('a lead whose node has an infinite vertex fails the anchors check', async (t) => {
  const infinite = leaded('infinite');
  infinite.nodes[0].points = [...FACE, [0, 0, Infinity]];
  const fx = await fixture(t, { infinite });
  assert.match(run(fx, ['infinite']).line('infinite'), /^FAIL {2}infinite {2}anchors: lead 1's node `lead` ends at \(NaN, NaN, NaN\)/);
});

test('a lead is the mesh of the node its anchor names', async (t) => {
  const fx = await fixture(t, {
    grouped: {
      part: { format: 1, part: 'grouped', pins: { a: [1] } },
      nodes: [{ name: 'lead', children: [{ name: 'lead-mesh', points: FACE }] }],
      anchors: { part: 'grouped', units: 'mm', anchors: [leg({ at: [99, 99, 99] })] },
    },
    decorated: {
      part: { format: 1, part: 'decorated', pins: { a: [1] } },
      nodes: [{ name: 'lead', points: FACE, children: [{ name: 'decoration', points: [[0, 0, -5]] }] }],
      anchors: { part: 'decorated', units: 'mm', anchors: [leg()] },
    },
  });
  const out = run(fx, ['decorated', 'grouped']);
  assert.match(out.line('grouped'), /anchors: lead 1's node `lead` has no vertices of its own/);
  assert.match(out.line('decorated'), /seating: node `decoration` reaches z = -5\.000 mm/);
});

test('the tip is the centroid of the vertices within 0.001 mm of the furthest', async (t) => {
  const spiked = leaded('spiked');
  spiked.nodes[0].points = [...FACE.slice(0, 3), [-0.5, 0.5, -1.0002]];
  const stepped = leaded('stepped');
  stepped.nodes[0].points = [...FACE, [0.5, 0.5, -0.998]];
  const fx = await fixture(t, { spiked, stepped });
  const out = run(fx, ['spiked', 'stepped']);
  assert.equal(out.status, 0, `${out.line('spiked')}\n${out.line('stepped')}`);
  assert.ok(asset(fx, 'spiked').checks.anchors.largest_miss_mm < 0.001);
  assert.equal(asset(fx, 'stepped').checks.anchors.largest_miss_mm, 0);
});

test("a child's pads are leads when its parent is composed", async (t) => {
  const fx = await fixture(t, {
    capped: {
      part: { format: 1, part: 'capped', pins: { a: [1] } },
      nodes: [
        { name: 'cap', points: [[-0.5, 0, -0.5], [0.5, 0, -0.5]] },
        { name: 'body', points: [[0, 0, 1]] },
      ],
      anchors: { part: 'capped', units: 'mm', anchors: [leg({ node: 'cap', at: [0, 0, -0.5], kind: 'pad' })] },
    },
    holder: board('holder', { c1: 'capped' }, { c1: { at: [0, 0, 0] } }),
  });
  const out = run(fx, ['capped', 'holder']);
  assert.equal(out.status, 0, out.line('holder'));
  assert.deepEqual(asset(fx, 'capped').kept_nodes, []);
  assert.equal(asset(fx, 'holder').checks.composition.lowest_node, 'pcb');
});

test("composition places a child's own children", async (t) => {
  const fx = await fixture(t, {
    block: BLOCK,
    mid: board('mid', { b: 'block' }, { b: { at: [10, 0, 0] } }),
    flat: board('flat', { m: 'mid' }, { m: { at: [0, 0, 0] } }),
    tilted: board('tilted', { m: 'mid' }, { m: { at: [0, 0, 0], rotate: [0, 20, 0] } }),
  });
  const out = run(fx, ['tilted', 'flat', 'mid', 'block']);
  assert.match(out.line('tilted'), /composition: placed by the layout, node `m\/b\/body` reaches z = -3\.420 mm/);
  assert.match(out.line('flat'), /^ok {4}flat {2}kept 0 nodes, 1, composed 3 draw calls/);
});

test('a child is composed from the commit its asset.json records', async (t) => {
  const fx = await fixture(t, { block: BLOCK, holder: board('holder', { u1: 'block' }, { u1: { at: [0, 0, 0] } }) });
  assert.equal(run(fx, ['block', 'holder']).status, 0);
  const sunk = await publish(fx, { block: { ...BLOCK, nodes: [{ name: 'body', points: [[0, 0, -3], [0, 0, 2]] }] } });
  const out = run(fx, ['holder'], sunk);
  assert.equal(out.status, 0, out.line('holder'));
  assert.equal(asset(fx, 'holder').checks.composition.z_mm, 0);
});

test('a part that holds itself fails composition', async (t) => {
  const fx = await fixture(t, { loop: { ...BLOCK, part: { format: 1, part: 'loop' } } });
  assert.equal(run(fx, ['loop']).status, 0);
  writePart(fx, 'loop', { format: 1, part: 'loop', parts: { self: { type: 'loop' } }, layout: { self: { at: [0, 0, 0] } } });
  const out = run(fx, ['loop']);
  assert.match(out.line('loop'), /^FAIL {2}loop {2}composition: child `self` closes a cycle: loop holds loop$/);
  assert.equal(out.stderr, '');
});

test('two children of one type, and a diamond, are not a cycle', async (t) => {
  const fx = await fixture(t, {
    block: BLOCK,
    twins: board('twins', { u1: 'block', u2: 'block', u3: 'block' }, { u1: { at: [0, 0, 0] }, u2: { at: [5, 0, 0] }, u3: { at: [10, 0, 0] } }),
    mid: board('mid', { b: 'block' }, { b: { at: [0, 0, 0] } }),
    diamond: board('diamond', { m1: 'mid', m2: 'mid' }, { m1: { at: [0, 0, 0] }, m2: { at: [10, 0, 0] } }),
  });
  const out = run(fx, ['block', 'twins', 'mid', 'diamond']);
  assert.equal(out.status, 0, out.stdout);
  assert.match(out.line('twins'), /^ok/);
  assert.match(out.line('diamond'), /^ok/);
});

test('a recorded commit the models checkout lacks fails its part and the parts that hold it', async (t) => {
  const fx = await fixture(t, { block: BLOCK, holder: board('holder', { u1: 'block' }, { u1: { at: [0, 0, 0] } }) });
  assert.equal(run(fx, ['block', 'holder']).status, 0);
  const missing = 'f'.repeat(40);
  const record = asset(fx, 'block');
  writeFileSync(join(fx.parts, 'block', 'asset.json'), JSON.stringify({ ...record, source: { ...record.source, commit: missing } }));
  const why = `source: \`block\`'s asset.json records models commit ${missing}, which ${fx.models} does not have; fetch it`;
  for (const ids of [['block'], ['holder']]) {
    const out = run(fx, ids, null);
    assert.equal(out.status, 1);
    assert.equal(out.line(ids[0]), `FAIL  ${ids[0]}  ${why}`);
    assert.equal(out.stderr, '');
  }
});

test('a part whose body and children are all leads has no lowest node', async (t) => {
  const pins = (id) => ({
    part: { format: 1, part: id, pins: { a: [1] } },
    nodes: [{ name: 'pin', points: [[0, 0, -2]] }],
    anchors: { part: id, units: 'mm', anchors: [leg({ node: 'pin', at: [0, 0, -2] })] },
  });
  const header = pins('header');
  const doubled = pins('doubled');
  doubled.part = { ...doubled.part, parts: { j1: { type: 'header' } }, layout: { j1: { at: [5, 0, 0] } } };
  const fx = await fixture(t, { header, doubled });
  const out = run(fx, ['header', 'doubled']);
  assert.equal(out.status, 0, out.line('doubled'));
  assert.deepEqual(asset(fx, 'doubled').checks.composition, { children: 1, lowest_node: null, z_mm: null, draw_calls: 2 });
});

test('a bad input fails its part, and the run goes on to the next part', async (t) => {
  const shapes = {
    'no-also': { also: undefined },
    'short-at': { at: [0, -1] },
    'short-axis': { axis: [0, -1] },
    'bad-kind': { kind: 'pin' },
  };
  const specs = Object.fromEntries(Object.entries(shapes).map(([id, anchor]) => [id, leaded(id, { anchor })]));
  specs['not-json'] = { ...leaded('not-json'), anchors: '{"part": ' };
  specs['empty-board'] = board('empty-board', { u1: 'block' }, { u1: { at: [0, 0, 0] } }, []);
  specs.block = BLOCK;
  specs['z-last'] = leaded('z-last');
  const fx = await fixture(t, specs);
  const out = run(fx, Object.keys(specs));
  assert.equal(out.status, 1);
  assert.match(out.line('no-also'), /anchors: anchor 1's `also` must be a list of node names, and is undefined/);
  assert.match(out.line('short-at'), /anchors: anchor 1's `at` must be three finite numbers, and is \[0,-1\]/);
  assert.match(out.line('short-axis'), /anchors: anchor 1's `axis` must be three finite numbers/);
  assert.match(out.line('bad-kind'), /anchors: anchor 1's `kind` must be leg, socket or pad, and is "pin"/);
  assert.match(out.line('not-json'), /^FAIL {2}not-json {2}error: /);
  assert.match(out.line('empty-board'), /merge: the merged scene has no nodes left to draw/);
  for (const id of ['no-also', 'short-at', 'short-axis', 'bad-kind', 'not-json', 'empty-board']) {
    assert.deepEqual(files(fx, id), ['part.jsonc'], id);
  }
  assert.match(out.line('z-last'), /^ok/);
  assert.deepEqual(files(fx, 'z-last'), ['asset.json', 'part.jsonc', 'z-last.anchors.json', 'z-last.glb']);
});

test('a part that fails keeps the files of its last good run', async (t) => {
  const fx = await fixture(t, { good: leaded('good') });
  assert.equal(run(fx, ['good']).status, 0);
  const before = files(fx, 'good').map((name) => readFileSync(join(fx.parts, 'good', name)));
  const moved = await publish(fx, { good: leaded('good', { anchor: { at: [9, 9, 9] } }) });
  const out = run(fx, ['good'], moved);
  assert.match(out.line('good'), /^FAIL {2}good {2}anchors: lead 1's node `lead` ends at/);
  assert.deepEqual(
    files(fx, 'good').map((name) => readFileSync(join(fx.parts, 'good', name))),
    before,
  );
});

test('with no --commit, a part with no asset.json is skipped', async (t) => {
  const fx = await fixture(t, { good: leaded('good'), wires: { part: { format: 1, part: 'wires', pins: { a: [1] } } } });
  assert.equal(run(fx, ['good']).status, 0);
  const out = run(fx, [], null);
  assert.equal(out.status, 0);
  assert.match(out.line('wires'), /^skip {2}wires {2}no --commit given and no asset.json records one/);
  assert.match(out.line('good'), /^ok/);
  assert.equal(existsSync(join(fx.parts, 'wires', 'asset.json')), false);
});

test('a --models that is not a git checkout is a usage error', async (t) => {
  const fx = await fixture(t, { good: leaded('good') });
  const { status, stderr } = spawnSync(process.execPath, [STEP, '--models', join(fx.parts, 'nowhere'), '--parts', fx.parts], { encoding: 'utf8', env });
  assert.equal(status, 2);
  assert.match(stderr, /^--models .*nowhere is not a git checkout/);
});

test('a --commit the models repo does not have is a usage error', async (t) => {
  const fx = await fixture(t, { good: leaded('good') });
  const { status, stderr } = spawnSync(process.execPath, [STEP, '--models', fx.models, '--parts', fx.parts, '--commit', 'no-such-rev'], {
    encoding: 'utf8',
    env,
  });
  assert.equal(status, 2);
  assert.match(stderr, /^--commit no-such-rev is not a commit of /);
});

test('passing --models and --local is a usage error', () => {
  const { status, stderr } = spawnSync(process.execPath, [STEP, '--models', 'x', '--local'], { encoding: 'utf8', env });
  assert.equal(status, 2);
  assert.match(stderr, /^pass either --models or --local/);
});

test('with no ids, a part with model.py is skipped', async (t) => {
  const fx = await fixture(t, { good: leaded('good'), wired: leaded('wired') });
  writeFileSync(join(fx.parts, 'wired', 'model.py'), '');
  const out = run(fx, []);
  assert.equal(out.status, 0);
  assert.match(out.line('wired'), /^skip {2}wired {2}built from model.py/);
  assert.match(out.line('good'), /^ok/);
  assert.equal(existsSync(join(fx.parts, 'wired', 'wired.glb')), false);
});

test('naming a part with model.py without --local fails', async (t) => {
  const fx = await fixture(t, { good: leaded('good') });
  writeFileSync(join(fx.parts, 'good', 'model.py'), '');
  const out = run(fx, ['good']);
  assert.equal(out.status, 1);
  assert.match(out.line('good'), /source: `good` is built from model.py\. Run scripts\/model\.sh good/);
});

test('--local reads the part folder and records model.py', async (t) => {
  const spec = leaded('good');
  const fx = await fixture(t, { good: spec });
  const folder = join(fx.parts, 'good');
  writeFileSync(join(folder, 'model.py'), '');
  writeFileSync(join(folder, 'good.glb'), await glb(spec.nodes));
  writeFileSync(join(folder, 'good.anchors.json'), JSON.stringify(spec.anchors));
  const { status, stdout } = spawnSync(process.execPath, [STEP, '--local', '--parts', fx.parts, 'good'], { encoding: 'utf8', env });
  assert.equal(status, 0, stdout);
  assert.match(stdout, /^ok\s+good /);
  const got = asset(fx, 'good');
  assert.equal(got.source.script, 'model.py');
  assert.equal(got.source.repo, undefined);
  assert.equal(got.files['good.glb'].bytes, readFileSync(join(folder, 'good.glb')).length);
  assert.equal(got.kept_nodes.includes('lead'), true);
});
