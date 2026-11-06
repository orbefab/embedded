// A GLB's nodes in the part frame: millimetres, +Z up. The part frame is the scene root's
// children, because the root carries only glTF's turn to Y-up, which the frame check verifies.

export const GLTF_TURN = [-Math.SQRT1_2, 0, 0, Math.SQRT1_2];

const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];

/** Column-major 4x4 product `a * b`. */
export function multiply(a, b) {
  const out = new Array(16).fill(0);
  for (let col = 0; col < 4; col++) {
    for (let row = 0; row < 4; row++) {
      for (let k = 0; k < 4; k++) out[col * 4 + row] += a[k * 4 + row] * b[col * 4 + k];
    }
  }
  return out;
}

export function apply(m, [x, y, z]) {
  return [
    m[0] * x + m[4] * y + m[8] * z + m[12],
    m[1] * x + m[5] * y + m[9] * z + m[13],
    m[2] * x + m[6] * y + m[10] * z + m[14],
  ];
}

/** A layout placement `{at, rotate}`: degrees about the parent's fixed X, then Y, then Z. */
export function placement({ at = [0, 0, 0], rotate = [0, 0, 0] }) {
  const [cx, cy, cz] = rotate.map((d) => Math.cos((d * Math.PI) / 180));
  const [sx, sy, sz] = rotate.map((d) => Math.sin((d * Math.PI) / 180));
  const rx = [1, 0, 0, 0, 0, cx, sx, 0, 0, -sx, cx, 0, 0, 0, 0, 1];
  const ry = [cy, 0, -sy, 0, 0, 1, 0, 0, sy, 0, cy, 0, 0, 0, 0, 1];
  const rz = [cz, sz, 0, 0, -sz, cz, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  const turn = multiply(rz, multiply(ry, rx));
  turn[12] = at[0];
  turn[13] = at[1];
  turn[14] = at[2];
  return turn;
}

/**
 * Every node under the scene root as `{name, parent, points, primitives}`, with its vertices
 * in the part frame in mm. `parent` indexes the returned list, and is null under the root.
 */
export function partNodes(doc) {
  const [root] = doc.getRoot().listScenes()[0].listChildren();
  const nodes = [];
  const walk = (node, parent, matrix) => {
    const world = multiply(matrix, node.getMatrix());
    const points = [];
    let primitives = 0;
    const mesh = node.getMesh();
    if (mesh) {
      for (const prim of mesh.listPrimitives()) {
        primitives++;
        const positions = prim.getAttribute('POSITION');
        const p = [0, 0, 0];
        for (let i = 0; i < positions.getCount(); i++) {
          positions.getElement(i, p);
          points.push(apply(world, p).map((v) => v * 1000));
        }
      }
    }
    const index = nodes.length;
    nodes.push({ name: node.getName(), parent, points, primitives });
    for (const child of node.listChildren()) walk(child, index, world);
  };
  for (const child of root.listChildren()) walk(child, null, IDENTITY);
  return nodes;
}

/** The centroid of the vertices furthest along `axis`. */
export function tip(points, axis) {
  const along = points.map((p) => p[0] * axis[0] + p[1] * axis[1] + p[2] * axis[2]);
  const far = Math.max(...along) - 1e-4;
  const chosen = points.filter((_, i) => along[i] >= far);
  return [0, 1, 2].map((k) => chosen.reduce((sum, p) => sum + p[k], 0) / chosen.length);
}

/** The indexes of the nodes named in `names` and of every node below them. */
export function subtree(nodes, names) {
  const inside = new Set();
  nodes.forEach((node, i) => {
    if (names.has(node.name) || inside.has(node.parent)) inside.add(i);
  });
  return inside;
}

/** The lowest vertex of any node outside `exempt`, as `{name, z}`, or null when every node is exempt. */
export function lowest(nodes, exempt) {
  let low = null;
  nodes.forEach((node, i) => {
    if (exempt.has(i)) return;
    for (const p of node.points) if (!low || p[2] < low.z) low = { name: node.name || '(merged)', z: p[2] };
  });
  return low;
}

export function box(nodes) {
  const min = [Infinity, Infinity, Infinity];
  const max = [-Infinity, -Infinity, -Infinity];
  for (const node of nodes) {
    for (const p of node.points) {
      for (let k = 0; k < 3; k++) {
        min[k] = Math.min(min[k], p[k]);
        max[k] = Math.max(max[k], p[k]);
      }
    }
  }
  return { min: min.map(mm), max: max.map(mm) };
}

export function distance(a, b) {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

/** A length for `asset.json`, rounded as the models script rounds anchors. */
export function mm(value) {
  return Math.round(value * 1e6) / 1e6 + 0;
}
