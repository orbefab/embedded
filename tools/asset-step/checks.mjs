// The asset step's checks and merge. Each check takes the part's state and returns what it
// measured for `asset.json`, or throws a `CheckFailed` that says why.

import { dedup, join, prune } from '@gltf-transform/functions';
import { GLTF_TURN, apply, box, distance, lowest, mm, partNodes, placement, subtree, tip } from './geometry.mjs';

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

export function anchorCheck(part, anchors, nodes) {
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
    if (!['leg', 'socket', 'pad'].includes(a.kind)) {
      fail('anchors', `lead ${a.lead} has kind \`${a.kind}\`, not leg, socket or pad`);
    }
    if (Math.abs(Math.hypot(...a.axis) - 1) > 1e-9) fail('anchors', `lead ${a.lead}'s axis is not a unit vector`);
    onceEach('anchors', nodes, [a.node, ...a.also], `lead ${a.lead}'s node and \`also\``);
    const at = tip(nodes.find((n) => n.name === a.node).points, a.axis);
    const miss = distance(at, a.at);
    if (miss > TIP_TOLERANCE) {
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

export function seating(anchors, nodes) {
  const low = lowest(nodes, subtree(nodes, leadNames(anchors)));
  if (!low) return { lowest_node: null, z_mm: null };
  if (low.z < -SEAT_TOLERANCE) {
    fail('seating', `node \`${low.name}\` reaches z = ${low.z.toFixed(3)} mm, below the seating plane; only a lead may`);
  }
  return { lowest_node: low.name, z_mm: mm(low.z) };
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
  const nodes = partNodes(doc);
  const kept = [...keep].sort();
  onceEach('merge', nodes, kept, 'the kept nodes');
  const drawCalls = nodes.reduce((sum, n) => sum + n.primitives, 0);
  return { kept, drawCalls, box: box(nodes), detail: { primitives_in: before, primitives_out: drawCalls } };
}

/**
 * Places every child's merged GLB by the part's layout and seats the whole part. A child's
 * pads merge away, so only its leg and socket nodes count as leads here.
 */
export function composition(part, bodyNodes, children) {
  const placed = bodyNodes.map((n) => ({ ...n, name: n.name || '(body)', lead: false }));
  let drawCalls = bodyNodes.reduce((sum, n) => sum + n.primitives, 0);
  for (const [role, type] of part.children) {
    const child = children.get(type);
    if (!child) fail('composition', `child \`${role}\` is a \`${type}\`, which has no asset step output`);
    const at = part.layout.get(role);
    if (!at) fail('composition', `child \`${role}\` has no placement in \`layout\``);
    const m = placement(at);
    const leads = subtree(child.nodes, leadNames(child.anchors));
    child.nodes.forEach((n, i) => {
      placed.push({
        name: `${role}/${n.name || '(merged)'}`,
        lead: leads.has(i),
        points: n.points.map((p) => apply(m, p)),
      });
    });
    drawCalls += child.drawCalls;
  }
  const exempt = new Set(placed.flatMap((n, i) => (n.lead ? [i] : [])));
  const low = lowest(placed, exempt);
  if (low.z < -SEAT_TOLERANCE) {
    fail('composition', `placed by the layout, \`${low.name}\` reaches z = ${low.z.toFixed(3)} mm; only a lead may go below z = 0`);
  }
  return {
    detail: { children: part.children.size, lowest_node: low.name, z_mm: mm(low.z) },
    drawCalls,
  };
}
