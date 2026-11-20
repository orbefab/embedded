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
const TOP_CAMERA_Y = 400;

/** How the tilted camera looks at a scene. */
export interface CameraStyle {
  /** Vertical field of view. */
  readonly fovDeg: number;
  /** The angle from straight down. */
  readonly tiltDeg: number;
  /** The turn about the vertical, from +Z toward +X. */
  readonly azimuthDeg: number;
  /** The narrowest span the view frames, so a lone part keeps room around it. */
  readonly minFrameMm: number;
}

export const DEFAULT_CAMERA: CameraStyle = {
  fovDeg: CAMERA_FOV_DEG,
  tiltDeg: CAMERA_TILT_DEG,
  azimuthDeg: 35,
  minFrameMm: MIN_FRAME_MM,
};

const radians = (deg: number) => (deg * Math.PI) / 180;

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

function basis(style: CameraStyle): Basis {
  const tilt = radians(style.tiltDeg);
  const azimuth = radians(style.azimuthDeg);
  const back: Vec3 = [Math.sin(tilt) * Math.sin(azimuth), Math.cos(tilt), Math.sin(tilt) * Math.cos(azimuth)];
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

export function tiltedOffset(style: CameraStyle = DEFAULT_CAMERA): Vec3 {
  const tilt = radians(style.tiltDeg);
  const azimuth = radians(style.azimuthDeg);
  const horizontal = CAMERA_DISTANCE_MM * Math.sin(tilt);
  return [horizontal * Math.sin(azimuth), CAMERA_DISTANCE_MM * Math.cos(tilt), horizontal * Math.cos(azimuth)];
}

function usableSpan(length: number): number {
  return Math.max(length - 2 * FRAME_INSET_PX, length / 2);
}

export function cameraPosition(view: CameraView, style: CameraStyle = DEFAULT_CAMERA): Vec3 {
  if (view.mode === "top") {
    return [view.target[0], TOP_CAMERA_Y, view.target[2]];
  }
  const { back } = basis(style);
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

export function frameView(
  mode: CameraMode,
  boxes: readonly Box[],
  viewport: Viewport,
  style: CameraStyle = DEFAULT_CAMERA
): CameraView {
  const points = boxes.flatMap(corners);
  const all = union(boxes);
  // A lone LED would otherwise fill the screen. Keep some room around it.
  const padX = Math.max(0, style.minFrameMm - (all.max[0] - all.min[0])) / 2;
  const padZ = Math.max(0, style.minFrameMm - (all.max[2] - all.min[2])) / 2;
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
  const { back, right, up } = basis(style);
  const tanHalf = Math.tan(radians(style.fovDeg) / 2);
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
