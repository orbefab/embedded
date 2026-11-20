// The viewer's looks: how the same scene is lit, shaded and outlined. A look is data that
// `look-stage.tsx` and `look-materials.ts` read. `current` is the viewer as it drew before looks,
// and is the default, so a page that names no look draws exactly what it drew before.
//
// Prototype: a page picks a look with `?look=<name>`, and the `L` key cycles them.

import { useSyncExternalStore } from "react";
import { type CameraStyle, DEFAULT_CAMERA } from "./camera-frame";

export type LookName = "current" | "studio" | "lite" | "toon" | "ink" | "clay";

export const LOOK_NAMES: readonly LookName[] = ["current", "studio", "lite", "toon", "ink", "clay"];

/** How a part's surfaces are redrawn. */
export type Surface =
  /** The GLB's materials as loaded. */
  | "as-loaded"
  /** Plastic is plastic and metal is metal, lit by an environment. */
  | "pbr"
  /** Cel shading in the part's own colours, softened. */
  | "toon"
  /** Cel shading in paper and ink, as orbefab.com's arm. Only signal colours keep their hue. */
  | "ink"
  /** Matte clay in the part's colours, lifted toward a warm white. */
  | "clay";

export interface Look {
  readonly name: LookName;
  readonly label: string;
  readonly surface: Surface;
  /** Ink lines: the silhouette's width in CSS px, and whether feature edges are drawn too. */
  readonly outline: { readonly px: number; readonly edges: boolean } | null;
  readonly lights: "bench" | "studio" | "toon" | "clay";
  /** An image-based light for reflections and soft fill, made in the page with no download. */
  readonly environment: number;
  /** `map`: a directional shadow map. `blob`: a soft analytic shadow under each part on the mat. */
  readonly shadows: "none" | "map" | "blob";
  /** Screen-space ambient occlusion, radius in mm. */
  readonly ao: { readonly radius: number; readonly intensity: number } | null;
  /** MSAA samples on the effect composer. 0 draws with no anti-aliasing. */
  readonly multisampling: number;
  /** The mat: the lit cutting-mat texture, a procedural grid fading into the background, or no
   * grid at all. */
  readonly mat: "texture" | "grid" | "plain";
  /** Hover outline colour. */
  readonly hover: string;
  readonly camera: CameraStyle;
}

/** From the front left and lower than `current`, so a connector's mouth, which faces -X in every
 * part frame, faces the camera, and a lone small part still fills some of the view. */
const FRONT_LEFT: CameraStyle = { fovDeg: 30, tiltDeg: 40, azimuthDeg: -35, minFrameMm: 16 };

/** orbefab.com's brand orange. */
export const BRAND = "#e85d04";

export const LOOKS: Readonly<Record<LookName, Look>> = {
  current: {
    name: "current",
    label: "Current",
    surface: "as-loaded",
    outline: null,
    lights: "bench",
    environment: 0,
    shadows: "none",
    ao: null,
    multisampling: 0,
    mat: "texture",
    hover: "#7dff67",
    camera: DEFAULT_CAMERA,
  },
  studio: {
    name: "studio",
    label: "Studio",
    surface: "pbr",
    outline: null,
    lights: "studio",
    environment: 0.55,
    shadows: "map",
    ao: { radius: 2.5, intensity: 2 },
    multisampling: 4,
    mat: "grid",
    hover: BRAND,
    camera: FRONT_LEFT,
  },
  // Studio for a machine with no GPU: about 3x studio's frame rate in software rendering, since
  // ambient occlusion and MSAA cost most of studio's time there and the shadow map little.
  lite: {
    name: "lite",
    label: "Studio lite",
    surface: "pbr",
    outline: null,
    lights: "studio",
    environment: 0.55,
    shadows: "map",
    ao: null,
    multisampling: 0,
    mat: "grid",
    hover: BRAND,
    camera: FRONT_LEFT,
  },
  toon: {
    name: "toon",
    label: "Toon",
    surface: "toon",
    outline: { px: 1.6, edges: true },
    lights: "toon",
    environment: 0,
    shadows: "blob",
    ao: null,
    multisampling: 4,
    mat: "grid",
    hover: BRAND,
    camera: { ...FRONT_LEFT, fovDeg: 24 },
  },
  ink: {
    name: "ink",
    label: "Ink",
    surface: "ink",
    outline: { px: 1.6, edges: true },
    lights: "toon",
    environment: 0,
    shadows: "blob",
    ao: null,
    multisampling: 4,
    mat: "plain",
    hover: BRAND,
    camera: { ...FRONT_LEFT, fovDeg: 24 },
  },
  clay: {
    name: "clay",
    label: "Clay",
    surface: "clay",
    outline: null,
    lights: "clay",
    environment: 0.35,
    shadows: "map",
    ao: { radius: 5, intensity: 3.2 },
    multisampling: 4,
    mat: "plain",
    hover: BRAND,
    camera: FRONT_LEFT,
  },
};

function isLookName(value: string | null): value is LookName {
  return value !== null && (LOOK_NAMES as readonly string[]).includes(value);
}

function fromUrl(): LookName {
  if (typeof window === "undefined") return "current";
  const asked = new URLSearchParams(window.location.search).get("look");
  return isLookName(asked) ? asked : "current";
}

let chosen: LookName | null = null;
const listeners = new Set<() => void>();

function read(): LookName {
  chosen ??= fromUrl();
  return chosen;
}

/** Shows `name` in every viewer on the page. */
export function setLook(name: LookName): void {
  chosen = name;
  for (const listener of listeners) listener();
}

/** The next look after the one shown. */
export function cycleLook(): void {
  const at = LOOK_NAMES.indexOf(read());
  setLook(LOOK_NAMES[(at + 1) % LOOK_NAMES.length] ?? "current");
}

function subscribe(onChange: () => void): () => void {
  listeners.add(onChange);
  return () => listeners.delete(onChange);
}

export function useLook(): Look {
  const name = useSyncExternalStore(subscribe, read, () => "current" as LookName);
  return LOOKS[name];
}
