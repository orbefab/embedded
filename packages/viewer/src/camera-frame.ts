// Adapted from maker-sandbox (MIT, Copyright 2026 Alwurts).
// Where the cameras sit to frame a box, in the viewer's world: mm, +Y up.

import type { Vec3 } from "./protocol";
import type { Box } from "./wire-geometry";

export const CAMERA_FOV_DEG = 45;
export const MIN_DISTANCE_MM = 28;
export const TOP_VIEW_ZOOM = 7;
const CAMERA_TILT_DEG = 25;
const CAMERA_DISTANCE_MM = 110;
const FRAME_INSET_PX = 56;
const MIN_FRAME_MM = 60;

const TILT = (CAMERA_TILT_DEG * Math.PI) / 180;
const AZIMUTH = (35 * Math.PI) / 180;
const TOP_CAMERA_Y = 400;

export type CameraMode = "tilted" | "top";

export interface Viewport {
  readonly width: number;
  readonly height: number;
}

export type CameraView =
  | { readonly mode: "tilted"; readonly target: Vec3; readonly distance: number }
  | { readonly mode: "top"; readonly target: Vec3; readonly zoom: number };

interface Basis {
  readonly back: Vec3;
  readonly right: Vec3;
  readonly up: Vec3;
}

function basis(): Basis {
  const back: Vec3 = [Math.sin(TILT) * Math.sin(AZIMUTH), Math.cos(TILT), Math.sin(TILT) * Math.cos(AZIMUTH)];
  const raw: Vec3 = [back[2], 0, -back[0]];
  const length = Math.hypot(raw[0], raw[1], raw[2]);
  const right: Vec3 = [raw[0] / length, raw[1] / length, raw[2] / length];
  const up: Vec3 = [
    back[1] * right[2] - back[2] * right[1],
    back[2] * right[0] - back[0] * right[2],
    back[0] * right[1] - back[1] * right[0],
  ];
  return { back, right, up };
}

function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

function corners(box: Box): Vec3[] {
  const points: Vec3[] = [];
  for (const x of [box.min[0], box.max[0]]) {
    for (const y of [box.min[1], box.max[1]]) {
      for (const z of [box.min[2], box.max[2]]) {
        points.push([x, y, z]);
      }
    }
  }
  return points;
}

export function tiltedOffset(): Vec3 {
  const horizontal = CAMERA_DISTANCE_MM * Math.sin(TILT);
  return [horizontal * Math.sin(AZIMUTH), CAMERA_DISTANCE_MM * Math.cos(TILT), horizontal * Math.cos(AZIMUTH)];
}

function usableSpan(length: number): number {
  return Math.max(length - 2 * FRAME_INSET_PX, length / 2);
}

export function cameraPosition(view: CameraView): Vec3 {
  if (view.mode === "top") {
    return [view.target[0], TOP_CAMERA_Y, view.target[2]];
  }
  const { back } = basis();
  return [
    view.target[0] + back[0] * view.distance,
    view.target[1] + back[1] * view.distance,
    view.target[2] + back[2] * view.distance,
  ];
}

/** The box around every box. */
export function union(boxes: readonly Box[]): Box {
  const along = (axis: 0 | 1 | 2) => boxes.flatMap((box) => [box.min[axis], box.max[axis]]);
  const [xs, ys, zs] = [along(0), along(1), along(2)];
  return {
    min: [Math.min(...xs), Math.min(...ys), Math.min(...zs)],
    max: [Math.max(...xs), Math.max(...ys), Math.max(...zs)],
  };
}

export function frameView(mode: CameraMode, boxes: readonly Box[], viewport: Viewport): CameraView {
  const points = boxes.flatMap(corners);
  const all = union(boxes);
  // A lone LED would otherwise fill the screen. Keep some room around it.
  const padX = Math.max(0, MIN_FRAME_MM - (all.max[0] - all.min[0])) / 2;
  const padZ = Math.max(0, MIN_FRAME_MM - (all.max[2] - all.min[2])) / 2;
  const min: Vec3 = [all.min[0] - padX, all.min[1], all.min[2] - padZ];
  const max: Vec3 = [all.max[0] + padX, all.max[1], all.max[2] + padZ];
  const target: Vec3 = [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2];
  const usableW = usableSpan(viewport.width);
  const usableH = usableSpan(viewport.height);
  if (mode === "top") {
    const spanX = Math.max(max[0] - min[0], 1);
    const spanZ = Math.max(max[2] - min[2], 1);
    return { mode: "top", target, zoom: Math.min(usableW / spanX, usableH / spanZ) };
  }
  const { back, right, up } = basis();
  const tanHalf = Math.tan((CAMERA_FOV_DEG * Math.PI) / 180 / 2);
  const height = Math.max(viewport.height, 1);
  const tanV = tanHalf * (usableH / height);
  const tanH = tanHalf * (usableW / height);
  let needed = MIN_DISTANCE_MM;
  for (const point of points) {
    const rel: Vec3 = [point[0] - target[0], point[1] - target[1], point[2] - target[2]];
    const ahead = dot(rel, back);
    needed = Math.max(needed, ahead + Math.abs(dot(rel, right)) / tanH, ahead + Math.abs(dot(rel, up)) / tanV);
  }
  return { mode: "tilted", target, distance: needed };
}
