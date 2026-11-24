//! Body footprints read once from each part's published `<id>.glb` and `<id>.anchors.json`,
//! composed through each part's `layout` into the scene's frame (mm, +Z up).
//!
//! Every published part ships its GLB, whose JSON chunk gives each mesh primitive's `POSITION`
//! accessor bounds in metres under the single root node carrying glTF's -90 degree turn about X.
//! Excluding leg lead meshes (and socket solder tails) leaves the body above the seating plane.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::Circuit;
use super::view::{Placement, placement};
use crate::model::Part;
use crate::source::FileId;

/// An axis-aligned bounding box in millimetres.
#[derive(Clone, Copy, Debug)]
pub(super) struct Aabb {
    pub(super) min: [f64; 3],
    pub(super) max: [f64; 3],
}

impl Aabb {
    /// Enclosing box after applying `m` to every corner of `self`.
    pub(super) fn transform(&self, m: &Mat4) -> Aabb {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for xi in [self.min[0], self.max[0]] {
            for yi in [self.min[1], self.max[1]] {
                for zi in [self.min[2], self.max[2]] {
                    let p = m.apply([xi, yi, zi]);
                    for k in 0..3 {
                        min[k] = min[k].min(p[k]);
                        max[k] = max[k].max(p[k]);
                    }
                }
            }
        }
        Aabb { min, max }
    }

    /// Overlap along X, Y and Z. Positive on all three axes means the two boxes intersect.
    pub(super) fn overlap(&self, other: &Aabb) -> [f64; 3] {
        [0, 1, 2].map(|k| self.max[k].min(other.max[k]) - self.min[k].max(other.min[k]))
    }

    /// The box enclosing all `boxes`, or `None` when empty.
    pub(super) fn union_all(boxes: &[Aabb]) -> Option<Aabb> {
        let mut iter = boxes.iter();
        let mut out = *iter.next()?;
        for b in iter {
            for k in 0..3 {
                out.min[k] = out.min[k].min(b.min[k]);
                out.max[k] = out.max[k].max(b.max[k]);
            }
        }
        Some(out)
    }
}

/// Column-major 4x4 affine transform.
#[derive(Clone, Copy, Debug)]
pub(super) struct Mat4([f64; 16]);

impl Mat4 {
    pub(super) const IDENTITY: Mat4 = Mat4([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]);

    pub(super) fn mul(&self, rhs: &Mat4) -> Mat4 {
        let mut out = [0.0; 16];
        for col in 0..4 {
            for row in 0..4 {
                out[col * 4 + row] = (0..4)
                    .map(|k| self.0[k * 4 + row] * rhs.0[col * 4 + k])
                    .sum();
            }
        }
        Mat4(out)
    }

    pub(super) fn apply(&self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let m = &self.0;
        [
            m[0] * x + m[4] * y + m[8] * z + m[12],
            m[1] * x + m[5] * y + m[9] * z + m[13],
            m[2] * x + m[6] * y + m[10] * z + m[14],
        ]
    }

    /// Rigid transform for a layout `Placement`: rotate about fixed X, then Y, then Z, then shift.
    pub(super) fn from_placement(p: Placement) -> Mat4 {
        let r = rotation_matrix(p.rotate);
        Mat4([
            r[0][0], r[1][0], r[2][0], 0.0, //
            r[0][1], r[1][1], r[2][1], 0.0, //
            r[0][2], r[1][2], r[2][2], 0.0, //
            p.at[0], p.at[1], p.at[2], 1.0,
        ])
    }

    /// Inverse of [`Mat4::from_placement`]: maps world points into `p`'s local frame.
    pub(super) fn inverse_placement(p: Placement) -> Mat4 {
        let r = rotation_matrix(p.rotate);
        // R^T * (-t)
        let tx = -(r[0][0] * p.at[0] + r[1][0] * p.at[1] + r[2][0] * p.at[2]);
        let ty = -(r[0][1] * p.at[0] + r[1][1] * p.at[1] + r[2][1] * p.at[2]);
        let tz = -(r[0][2] * p.at[0] + r[1][2] * p.at[1] + r[2][2] * p.at[2]);
        Mat4([
            r[0][0], r[0][1], r[0][2], 0.0, //
            r[1][0], r[1][1], r[1][2], 0.0, //
            r[2][0], r[2][1], r[2][2], 0.0, //
            tx, ty, tz, 1.0,
        ])
    }

    /// A glTF node's local transform (`matrix`, else `T * R * S`).
    fn from_gltf_node(node: &Value) -> Mat4 {
        if let Some(items) = node.get("matrix").and_then(Value::as_array)
            && items.len() == 16
            && let Some(vals) = items.iter().map(Value::as_f64).collect::<Option<Vec<_>>>()
            && let Ok(arr) = vals.try_into()
        {
            return Mat4(arr);
        }
        let [tx, ty, tz] = three(&node["translation"]).unwrap_or([0.0; 3]);
        let [qx, qy, qz, qw] = four(&node["rotation"]).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let [sx, sy, sz] = three(&node["scale"]).unwrap_or([1.0; 3]);
        let (x2, y2, z2) = (qx + qx, qy + qy, qz + qz);
        let (xx, xy, xz) = (qx * x2, qx * y2, qx * z2);
        let (yy, yz, zz) = (qy * y2, qy * z2, qz * z2);
        let (wx, wy, wz) = (qw * x2, qw * y2, qw * z2);
        Mat4([
            (1.0 - (yy + zz)) * sx,
            (xy + wz) * sx,
            (xz - wy) * sx,
            0.0,
            (xy - wz) * sy,
            (1.0 - (xx + zz)) * sy,
            (yz + wx) * sy,
            0.0,
            (xz + wy) * sz,
            (yz - wx) * sz,
            (1.0 - (xx + yy)) * sz,
            0.0,
            tx,
            ty,
            tz,
            1.0,
        ])
    }
}

fn rotation_matrix([rx, ry, rz]: [f64; 3]) -> [[f64; 3]; 3] {
    let (sx, cx) = libm::sincos(rx.to_radians());
    let (sy, cy) = libm::sincos(ry.to_radians());
    let (sz, cz) = libm::sincos(rz.to_radians());
    [
        [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
        [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
        [-sy, cy * sx, cy * cx],
    ]
}

fn three(value: &Value) -> Option<[f64; 3]> {
    let items = value.as_array()?;
    match items[..] {
        [ref x, ref y, ref z] => Some([x.as_f64()?, y.as_f64()?, z.as_f64()?]),
        _ => None,
    }
}

fn four(value: &Value) -> Option<[f64; 4]> {
    let items = value.as_array()?;
    match items[..] {
        [ref x, ref y, ref z, ref w] => Some([x.as_f64()?, y.as_f64()?, z.as_f64()?, w.as_f64()?]),
        _ => None,
    }
}

/// Body boxes of one GLB in its part frame (mm, +Z up), read from the JSON chunk's `POSITION`
/// accessors. Excludes `leg` anchor `node` meshes (the leads below the body) and `socket` anchor
/// `also` meshes (solder tails under a board), while keeping header housings and body meshes.
fn glb_body_boxes(glb: &[u8], anchors: &[Value]) -> Vec<Aabb> {
    if glb.len() < 20 || &glb[..4] != b"glTF" || &glb[16..20] != b"JSON" {
        return Vec::new();
    }
    let len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
    let Some(chunk) = glb.get(20..20 + len) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_slice::<Value>(chunk) else {
        return Vec::new();
    };
    let Some(nodes) = json.get("nodes").and_then(Value::as_array) else {
        return Vec::new();
    };
    let meshes = json
        .get("meshes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let accessors = json
        .get("accessors")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let scene_idx = json.get("scene").and_then(Value::as_u64).unwrap_or(0) as usize;
    let Some(root_idx) = json
        .get("scenes")
        .and_then(|s| s.get(scene_idx))
        .and_then(|s| s.get("nodes"))
        .and_then(|n| n.get(0))
        .and_then(Value::as_u64)
        .map(|i| i as usize)
    else {
        return Vec::new();
    };
    let Some(root) = nodes.get(root_idx) else {
        return Vec::new();
    };

    let mut skip: BTreeSet<&str> = BTreeSet::new();
    for anchor in anchors {
        match anchor.get("kind").and_then(Value::as_str) {
            Some("leg") => {
                if let Some(name) = anchor.get("node").and_then(Value::as_str) {
                    skip.insert(name);
                }
            }
            Some("socket") => {
                for also in anchor
                    .get("also")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(name) = also.as_str() {
                        skip.insert(name);
                    }
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    let mut stack: Vec<(usize, Mat4)> = root
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_u64().map(|i| (i as usize, Mat4::IDENTITY)))
        .collect();

    while let Some((idx, parent_m)) = stack.pop() {
        let Some(node) = nodes.get(idx) else {
            continue;
        };
        let world = parent_m.mul(&Mat4::from_gltf_node(node));
        let skipped = node
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| skip.contains(name));
        if !skipped && let Some(mesh_idx) = node.get("mesh").and_then(Value::as_u64) {
            let mut min = [f64::INFINITY; 3];
            let mut max = [f64::NEG_INFINITY; 3];
            let mut found = false;
            if let Some(prims) = meshes
                .get(mesh_idx as usize)
                .and_then(|m| m.get("primitives"))
                .and_then(Value::as_array)
            {
                for prim in prims {
                    let Some(pos_idx) = prim
                        .get("attributes")
                        .and_then(|a| a.get("POSITION"))
                        .and_then(Value::as_u64)
                    else {
                        continue;
                    };
                    let Some(acc) = accessors.get(pos_idx as usize) else {
                        continue;
                    };
                    let (Some(lo), Some(hi)) = (three(&acc["min"]), three(&acc["max"])) else {
                        continue;
                    };
                    for xi in [lo[0], hi[0]] {
                        for yi in [lo[1], hi[1]] {
                            for zi in [lo[2], hi[2]] {
                                let p = world.apply([xi, yi, zi]);
                                found = true;
                                for k in 0..3 {
                                    let mm = p[k] * 1000.0;
                                    min[k] = min[k].min(mm);
                                    max[k] = max[k].max(mm);
                                }
                            }
                        }
                    }
                }
            }
            if found {
                out.push(Aabb { min, max });
            }
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                if let Some(c) = child.as_u64() {
                    stack.push((c as usize, world));
                }
            }
        }
    }
    out
}

/// Cached local body boxes per part file, so each part's GLB is read at most once.
#[derive(Default)]
pub(super) struct BoxCache {
    by_file: BTreeMap<FileId, Vec<Aabb>>,
}

impl Circuit<'_> {
    fn glb_bytes(&self, part: &Part) -> Option<Vec<u8>> {
        let id = &part.label;
        let rel = format!("{id}.glb");
        let file = &self.project.sources.file(part.file).path;
        if let Some(folder) = file.parent() {
            let path = folder.join(&rel);
            if let Ok(bytes) = self.project.files.read(&path) {
                return Some(bytes);
            }
        }
        let root = self.project.sources.root();
        self.project.store.read(self.project.files, root, id, &rel)
    }

    fn part_body_boxes(&self, part: &Part, cache: &mut BoxCache) -> Vec<Aabb> {
        if let Some(cached) = cache.by_file.get(&part.file) {
            return cached.clone();
        }
        let boxes = match self.glb_bytes(part) {
            Some(bytes) => {
                let anchors = self.anchors_of(part);
                glb_body_boxes(&bytes, &anchors)
            }
            None => Vec::new(),
        };
        cache.by_file.insert(part.file, boxes.clone());
        boxes
    }

    /// Every body box under `inst` (its own GLB and all children recursively, placed by each
    /// parent's `layout`), transformed by `world`.
    pub(super) fn instance_boxes(
        &self,
        inst: usize,
        world: &Mat4,
        trees: &mut BTreeMap<FileId, Value>,
        cache: &mut BoxCache,
    ) -> Vec<Aabb> {
        let mut out = Vec::new();
        let mut stack = vec![(inst, *world)];
        while let Some((at, matrix)) = stack.pop() {
            let Some(part) = self.part(at) else {
                continue;
            };
            for local in self.part_body_boxes(part, cache) {
                out.push(local.transform(&matrix));
            }
            if !self.insts[at].children.is_empty() {
                let tree = self.tree(trees, part.file);
                for (name, &child) in &self.insts[at].children {
                    let place = placement(&tree["layout"][name.as_str()]).unwrap_or(Placement {
                        at: [0.0; 3],
                        rotate: [0.0; 3],
                    });
                    stack.push((child, matrix.mul(&Mat4::from_placement(place))));
                }
            }
        }
        out
    }
}
