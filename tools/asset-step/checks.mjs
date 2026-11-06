// The asset step's checks and merge. Each check takes the part's state and returns what it
// measured for `asset.json`, or throws a `CheckFailed` that says why.

import { dedup, join, prune } from '@gltf-transform/functions';
import { GLTF_TURN, IDENTITY, apply, box, distance, lowest, mm, multiply, partNodes, placement, tip } from './geometry.mjs';

const TIP_TOLERANCE = 0.01;
const SEAT_TOLERANCE = 0.01;
const KEPT_KINDS = new Set(['leg', 'socket']);

export class CheckFailed extends Error {
  constructor(check, why) {
    super(why);
    this.check = check;
  }
}

function fail(check, why) {
  throw new CheckFailed(check, why);
}

function leadList(leads) {
  return `${leads.length === 1 ? 'lead' : 'leads'} ${leads.join(', ')}`;
}

function counts(nodes) {
  const count = new Map();
  for (const { name } of nodes) count.set(name, (count.get(name) ?? 0) + 1);
  return count;
}

function onceEach(check, nodes, names, what) {
  const count = counts(nodes);
  const bad = names.filter((name) => count.get(name) !== 1);
  if (bad.length) {
    const found = bad.map((name) => `\`${name}\` ${count.get(name) ?? 0} times`).join(', ');
    fail(check, `${what} must each name one node of the GLB: ${found}`);
  }
}

export function frame(doc) {
  const roots = doc.getRoot().listScenes()[0]?.listChildren() ?? [];
  if (roots.length !== 1) fail('frame', `the scene has ${roots.length} root nodes, not 1`);
  const [root] = roots;
  const turned = root.getRotation().every((q, i) => Math.abs(q - GLTF_TURN[i]) <= 1e-9);
  const moved = root.getTranslation().some((v) => v !== 0);
  const scaled = root.getScale().some((v) => v !== 1);
  if (!turned || moved || scaled) {
    fail(
      'frame',
      `the root must carry only glTF's -90 degree turn about X, and has rotation ` +
        `${JSON.stringify(root.getRotation())}, translation ${JSON.stringify(root.getTranslation())}, ` +
        `scale ${JSON.stringify(root.getScale())}`,
    );
  }
  if (root.getMesh()) fail('frame', 'the root node draws a mesh; only its children may');
  return { root_rotation: root.getRotation().map(mm) };
}

const finite3 = (v) => Array.isArray(v) && v.length === 3 && v.every(Number.isFinite);
const nodeName = (v) => typeof v === 'string' && v !== '';

const ANCHOR_FIELDS = [
  ['lead', Number.isInteger, 'an integer'],
  ['node', nodeName, 'a node name'],
  ['also', (v) => Array.isArray(v) && v.every(nodeName), 'a list of node names'],
  ['at', finite3, 'three finite numbers'],
  ['axis', finite3, 'three finite numbers'],
  ['kind', (v) => ['leg', 'socket', 'pad'].includes(v), 'leg, socket or pad'],
];

function anchorShape(anchors) {
  if (!Array.isArray(anchors?.anchors)) fail('anchors', 'the anchors file has no `anchors` list');
  anchors.anchors.forEach((a, i) => {
    for (const [field, ok, what] of ANCHOR_FIELDS) {
      if (!ok(a?.[field])) fail('anchors', `anchor ${i + 1}'s \`${field}\` must be ${what}, and is ${JSON.stringify(a?.[field])}`);
    }
  });
}

/**
 * A lead is the mesh of the node an anchor names, so its tip is taken from that node's own
 * vertices, not its children's.
 */
export function anchorCheck(part, anchors, nodes) {
  anchorShape(anchors);
  if (anchors.part !== part.id || anchors.units !== 'mm') {
    fail('anchors', `the anchors file names part \`${anchors.part}\` in \`${anchors.units}\`, not \`${part.id}\` in mm`);
  }
  const leads = anchors.anchors.map((a) => a.lead);
  const twice = leads.filter((lead, i) => leads.indexOf(lead) !== i);
  if (twice.length) fail('anchors', `${leadList([...new Set(twice)])} must have one anchor each, and have more`);
  const missing = [...part.leads].filter((lead) => !leads.includes(lead));
  if (missing.length) fail('anchors', `${leadList(missing)} in \`pins\` must have an anchor, and has none`);
  const extra = leads.filter((lead) => !part.leads.has(lead));
  if (extra.length) fail('anchors', `the anchors have ${leadList(extra)}, which \`pins\` does not have`);

  let largest = 0;
  for (const a of anchors.anchors) {
    if (Math.abs(Math.hypot(...a.axis) - 1) > 1e-9) fail('anchors', `lead ${a.lead}'s axis is not a unit vector`);
    onceEach('anchors', nodes, [a.node, ...a.also], `lead ${a.lead}'s node and \`also\``);
    const { points } = nodes.find((n) => n.name === a.node);
    if (!points.length) {
      fail('anchors', `lead ${a.lead}'s node \`${a.node}\` has no vertices of its own; a lead's mesh must be on the node its anchor names`);
    }
    const at = tip(points, a.axis);
    const miss = distance(at, a.at);
    if (!(miss <= TIP_TOLERANCE)) {
      fail(
        'anchors',
        `lead ${a.lead}'s node \`${a.node}\` ends at (${at.map((v) => v.toFixed(3)).join(', ')}), ` +
          `${miss.toFixed(4)} mm from its anchor at (${a.at.join(', ')}); the tolerance is ${TIP_TOLERANCE} mm`,
      );
    }
    largest = Math.max(largest, miss);
  }
  return { anchors: anchors.anchors.length, largest_miss_mm: mm(largest) };
}

function leadNames(anchors) {
  return new Set(anchors.anchors.flatMap((a) => [a.node, ...a.also]));
}

/** Fails `check` when a node outside `exempt` reaches below the seating plane. */
function seat(check, nodes, exempt, context) {
  const low = lowest(nodes, exempt);
  if (!low) return { lowest_node: null, z_mm: null };
  if (!Number.isFinite(low.z)) fail(check, `${context}node \`${low.name}\` has a vertex that is not a finite number`);
  if (!(low.z >= -SEAT_TOLERANCE)) {
    fail(check, `${context}node \`${low.name}\` reaches z = ${low.z.toFixed(3)} mm, below the seating plane; only a lead may`);
  }
  return { lowest_node: low.name, z_mm: mm(low.z) };
}

/** Only the nodes an anchor names are leads. A node under a lead is not one. */
export function seating(anchors, nodes) {
  const leads = leadNames(anchors);
  return seat('seating', nodes, new Set(nodes.flatMap((n, i) => (leads.has(n.name) ? [i] : []))), '');
}

export function names(part, nodes) {
  const named = [...part.viewNodes, ...part.jointNodes];
  onceEach('names', nodes, named, 'the nodes `view` and `joints` name');
  return { nodes: [...new Set(named)].sort() };
}

/**
 * Keeps the nodes `view` and `joints` name and the `node` and `also` of every leg or socket
 * anchor. Every other node and every mesh loses its name, so `join` merges them into one mesh
 * per material. `dedup` then shares identical meshes, such as a header's 15 pins.
 */
export async function merge(doc, part, anchors) {
  const keep = new Set([...part.viewNodes, ...part.jointNodes]);
  for (const a of anchors.anchors) {
    if (KEPT_KINDS.has(a.kind)) [a.node, ...a.also].forEach((name) => keep.add(name));
  }
  const before = partNodes(doc).reduce((sum, n) => sum + n.primitives, 0);
  for (const mesh of doc.getRoot().listMeshes()) mesh.setName('');
  for (const node of doc.getRoot().listNodes()) if (!keep.has(node.getName())) node.setName('');
  await doc.transform(join({ keepNamed: true }), dedup(), prune());
  if (!doc.getRoot().listScenes()[0]?.listChildren().length) fail('merge', 'the merged scene has no nodes left to draw');
  const nodes = partNodes(doc);
  const kept = [...keep].sort();
  onceEach('merge', nodes, kept, 'the kept nodes');
  const drawCalls = nodes.reduce((sum, n) => sum + n.primitives, 0);
  return { kept, drawCalls, box: box(nodes), detail: { primitives_in: before, primitives_out: drawCalls } };
}

/**
 * Places every child's checked source model by the part's layout, and each child's own children
 * by the child's layout, then seats the whole part. `model` and each entry of `models` are
 * `{part, nodes, anchors, drawCalls}`, with the nodes from the source GLB, so a pad's node still
 * has its name and is a lead here as it is in `seating`.
 */
export function composition(model, models) {
  const placed = [];
  const exempt = new Set();
  let drawCalls = 0;
  const place = ({ part, nodes, anchors, drawCalls: calls }, matrix, prefix) => {
    const leads = leadNames(anchors);
    for (const n of nodes) {
      if (leads.has(n.name)) exempt.add(placed.length);
      placed.push({ name: `${prefix}${n.name || '(unnamed)'}`, points: n.points.map((p) => apply(matrix, p)) });
    }
    drawCalls += calls;
    for (const [role, type] of part.children) {
      const child = models.get(type);
      if (!child) fail('composition', `child \`${prefix}${role}\` is a \`${type}\`, which has no asset step output`);
      const at = part.layout.get(role);
      if (!at) fail('composition', `child \`${prefix}${role}\` has no placement in \`layout\``);
      place(child, multiply(matrix, placement(at)), `${prefix}${role}/`);
    }
  };
  place(model, IDENTITY, '');
  return {
    detail: { children: model.part.children.size, ...seat('composition', placed, exempt, 'placed by the layout, ') },
    drawCalls,
  };
}
