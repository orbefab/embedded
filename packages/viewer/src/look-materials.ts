// A look's surfaces: the materials a part is redrawn with, and its ink lines. Applied to a built
// scene in place and undone by applying another look, so switching never rebuilds the scene.

import {
  BackSide,
  BufferGeometry,
  Color,
  DataTexture,
  EdgesGeometry,
  Float32BufferAttribute,
  LineBasicMaterial,
  LineSegments,
  type Material,
  Mesh,
  MeshStandardMaterial,
  MeshToonMaterial,
  NearestFilter,
  type Object3D,
  RedFormat,
  ShaderMaterial,
  SRGBColorSpace,
  type Texture,
  Vector2,
} from "three";
import { mergeVertices } from "three/addons/utils/BufferGeometryUtils.js";
import type { Look } from "./look";

/** What a surface is made of. The GLBs say nothing about it, since OCCT's writer leaves out
 * glTF's metallic and roughness factors and every material defaults to rough metal. The viewer
 * reads it from the colour: the cool light greys the models use for tin and steel, and their
 * two golds. */
export type Finish = "metal" | "gold" | "plastic";

const GOLDS = ["#d9b44a", "#c8a85a"].map((hex) => new Color(hex));

export function finishOf(color: Color): Finish {
  const [r, g, b] = srgbBytes(color);
  const spread = Math.max(r, g, b) - Math.min(r, g, b);
  if (r >= 178 && r <= 216 && b >= r && spread <= 16) return "metal";
  for (const gold of GOLDS) {
    const [gr, gg, gb] = srgbBytes(gold);
    if (Math.abs(r - gr) <= 3 && Math.abs(g - gg) <= 3 && Math.abs(b - gb) <= 3) return "gold";
  }
  return "plastic";
}

function srgbBytes(color: Color): [number, number, number] {
  const hex = color.getHex(SRGBColorSpace);
  return [(hex >> 16) & 255, (hex >> 8) & 255, hex & 255];
}

/** The colours a look draws in, for one theme. */
export interface Ink {
  /** Lines and silhouettes. */
  readonly line: string;
  /** Ink's paper, its dark parts, and its metal. */
  readonly paper: string;
  readonly dark: string;
  readonly metal: string;
}

export function inkOf(dark: boolean): Ink {
  // The same paper and ink as orbefab.com's arm (so101/scene.ts, paletteOf).
  return dark
    ? { line: "#0b0b0b", paper: "#d9d9da", dark: "#3a3a3c", metal: "#b4b4b8" }
    : { line: "#26262a", paper: "#fbfbfb", dark: "#2a2a2c", metal: "#d4d4d8" };
}

const hsl = { h: 0, s: 0, l: 0 };

/** The part's colour, softened for cel shading: blacks lifted off the ink, whites kept off the
 * paper, saturation eased. */
function soft(source: Color, dark: boolean, into: Color): Color {
  source.getHSL(hsl, SRGBColorSpace);
  const l = dark ? 0.2 + hsl.l * 0.66 : 0.17 + hsl.l * 0.79;
  return into.setHSL(hsl.h, hsl.s * 0.82, l, SRGBColorSpace);
}

/** Paper and ink, with a trace of the hue on a saturated surface such as a PCB. */
function inked(source: Color, finish: Finish, ink: Ink, into: Color): Color {
  if (finish !== "plastic") return into.set(ink.metal);
  source.getHSL(hsl, SRGBColorSpace);
  if (hsl.l < 0.3 && hsl.s < 0.5) return into.set(ink.dark);
  into.set(ink.paper);
  return hsl.s > 0.3 ? into.lerp(source, 0.16) : into;
}

/** Clay: the hue kept, lifted and eased toward a warm white. */
function clayed(source: Color, finish: Finish, dark: boolean, into: Color): Color {
  if (finish !== "plastic") return into.set(dark ? "#9c9790" : "#cfc9c0");
  source.getHSL(hsl, SRGBColorSpace);
  const l = dark ? 0.26 + hsl.l * 0.55 : 0.34 + hsl.l * 0.56;
  into.setHSL(hsl.h, hsl.s * 0.62, l, SRGBColorSpace);
  return into.lerp(new Color(dark ? "#b9b2a8" : "#f3ede4"), 0.18);
}

/** The homepage arm's three-step ramp. */
function toonRamp(): DataTexture {
  const ramp = new DataTexture(new Uint8Array([150, 205, 255]), 3, 1, RedFormat);
  ramp.minFilter = NearestFilter;
  ramp.magFilter = NearestFilter;
  ramp.needsUpdate = true;
  return ramp;
}

/** A back-face hull pushed out along its smooth normals by a constant width on screen. */
function hullMaterial(color: string): ShaderMaterial {
  return new ShaderMaterial({
    side: BackSide,
    uniforms: { color: { value: new Color(color) }, width: { value: 1.5 }, resolution: { value: new Vector2(1, 1) } },
    vertexShader: `
      uniform float width;
      uniform vec2 resolution;
      void main() {
        vec4 clip = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
        vec4 clipN = projectionMatrix * vec4(normalize(normalMatrix * normal), 0.0);
        vec2 screen = (clipN.xy * clip.w - clip.xy * clipN.w) * resolution;
        float len = length(screen);
        if (len > 1e-6) clip.xy += (screen / len) * (2.0 * width / resolution) * clip.w;
        gl_Position = clip;
      }`,
    fragmentShader: `
      uniform vec3 color;
      void main() {
        gl_FragColor = vec4(color, 1.0);
        #include <colorspace_fragment>
      }`,
  });
}

interface Decor {
  readonly smooth: BufferGeometry;
  readonly edges: BufferGeometry;
}

/** Each mesh geometry's hull and edges, made once per scene and shared by every look and instance
 * that draws it. */
export class DecorCache {
  private readonly made = new Map<BufferGeometry, Decor>();

  get(geometry: BufferGeometry): Decor {
    let found = this.made.get(geometry);
    if (found === undefined) {
      found = { smooth: smoothCopy(geometry), edges: featureEdges(geometry, 0.35) };
      this.made.set(geometry, found);
    }
    return found;
  }

  dispose(): void {
    for (const { smooth, edges } of this.made.values()) {
      smooth.dispose();
      edges.dispose();
    }
    this.made.clear();
  }
}

/** Feature edges sharper than 35 degrees, without the segments shorter than `minMm`. The GLB is
 * in metres, and EdgesGeometry rounds to 1e-4, so the edges are found in mm. */
function featureEdges(geometry: BufferGeometry, minMm: number): BufferGeometry {
  const mm = geometry.clone().scale(1000, 1000, 1000);
  const edges = new EdgesGeometry(mm, 35);
  mm.dispose();
  const at = edges.getAttribute("position");
  const keep: number[] = [];
  for (let i = 0; i < at.count; i += 2) {
    const dx = at.getX(i) - at.getX(i + 1);
    const dy = at.getY(i) - at.getY(i + 1);
    const dz = at.getZ(i) - at.getZ(i + 1);
    if (dx * dx + dy * dy + dz * dz < minMm * minMm) continue;
    keep.push(at.getX(i), at.getY(i), at.getZ(i), at.getX(i + 1), at.getY(i + 1), at.getZ(i + 1));
  }
  edges.dispose();
  return new BufferGeometry().setAttribute("position", new Float32BufferAttribute(keep, 3)).scale(0.001, 0.001, 0.001);
}

/** `geometry` with its vertices welded and smooth normals, for a hull with no gaps at creases. */
export function smoothCopy(geometry: BufferGeometry): BufferGeometry {
  // A GLB's positions may be interleaved, which mergeVertices cannot write, so copy them out.
  const at = geometry.getAttribute("position");
  const positions = new Float32Array(at.count * 3);
  for (let i = 0; i < at.count; i++) {
    positions[i * 3] = at.getX(i);
    positions[i * 3 + 1] = at.getY(i);
    positions[i * 3 + 2] = at.getZ(i);
  }
  const bare = new BufferGeometry().setAttribute("position", new Float32BufferAttribute(positions, 3));
  if (geometry.index !== null) bare.setIndex(geometry.index.clone());
  const smooth = mergeVertices(bare, 1e-7);
  smooth.computeVertexNormals();
  return smooth;
}

const noRaycast = () => {};

/** Materials and lines for one look in one theme. Shared by every part and wire it draws. */
export interface LookKit {
  readonly look: Look;
  readonly dark: boolean;
  /** The material `source` is redrawn with. `signal` is a surface whose colour carries
   * meaning: an LED, a tint, a resistor band, a wire. */
  surface(source: Material, signal: boolean): Material;
  /** A wire's insulation. */
  wire(colour: string): Material;
  readonly housing: Material;
  readonly hull: ShaderMaterial | null;
  readonly line: LineBasicMaterial | null;
  setResolution(width: number, height: number, pixelRatio: number): void;
  dispose(): void;
}

/** `environment` is the image-based light for a look that has one. */
export function createKit(look: Look, dark: boolean, environment: Texture | null = null): LookKit {
  const ink = inkOf(dark);
  const ramp = look.surface === "toon" || look.surface === "ink" ? toonRamp() : null;
  const made = new Map<string, Material>();
  const owned: Material[] = [];
  const hull = look.outline === null ? null : hullMaterial(ink.line);
  const line = look.outline?.edges === true ? new LineBasicMaterial({ color: ink.line }) : null;

  const toon = (color: Color): MeshToonMaterial => {
    const material = new MeshToonMaterial({ color, gradientMap: ramp });
    material.polygonOffset = true;
    material.polygonOffsetFactor = 1;
    material.polygonOffsetUnits = 1;
    return material;
  };

  const derive = (source: Material, signal: boolean): Material => {
    if (!(source instanceof MeshStandardMaterial) || look.surface === "as-loaded") return source;
    const finish = finishOf(source.color);
    const color = new Color();
    let next: MeshStandardMaterial | MeshToonMaterial;
    switch (look.surface) {
      case "pbr": {
        next = source.clone();
        const metal = finish !== "plastic" && !signal;
        next.metalness = metal ? 1 : 0;
        // Black epoxy and moulded plastic keep a soft sheen, so a black part still shows its form.
        source.color.getHSL(hsl, SRGBColorSpace);
        next.roughness = metal ? (finish === "gold" ? 0.32 : 0.36) : signal ? 0.28 : hsl.l < 0.2 ? 0.4 : 0.52;
        next.envMap = environment;
        next.envMapIntensity = look.environment * (metal ? 1.8 : 0.5);
        break;
      }
      case "toon":
        next = toon(finish === "plastic" || signal ? soft(source.color, dark, color) : color.set(dark ? "#a9abb0" : "#d6d8dc"));
        break;
      case "ink":
        next = toon(signal ? soft(source.color, dark, color) : inked(source.color, finish, ink, color));
        break;
      case "clay":
        next = new MeshStandardMaterial({ color: clayed(source.color, finish, dark, color), roughness: 0.92, metalness: 0 });
        next.envMap = environment;
        next.envMapIntensity = look.environment;
        if (signal) next.color.copy(soft(source.color, dark, color));
        break;
    }
    next.side = source.side;
    next.shadowSide = BackSide;
    next.emissive.copy(source.emissive);
    next.emissiveIntensity = source.emissiveIntensity;
    next.toneMapped = source.toneMapped;
    next.name = source.name;
    owned.push(next);
    return next;
  };

  return {
    look,
    dark,
    surface(source, signal) {
      if (look.surface === "as-loaded") return source;
      const key = `${source.uuid}:${signal}`;
      let found = made.get(key);
      if (found === undefined) {
        found = derive(source, signal);
        made.set(key, found);
      }
      return found;
    },
    wire(colour) {
      const key = `wire:${colour}`;
      let found = made.get(key);
      if (found === undefined) {
        const source = new MeshStandardMaterial({ color: colour });
        owned.push(source);
        found = look.surface === "as-loaded" ? source : derive(source, true);
        if (found instanceof MeshStandardMaterial && look.surface === "pbr") found.roughness = 0.38;
        made.set(key, found);
      }
      return found;
    },
    housing: (() => {
      const source = new MeshStandardMaterial({ color: "#161616", roughness: 0.55 });
      owned.push(source);
      return look.surface === "as-loaded" ? source : derive(source, false);
    })(),
    hull,
    line,
    setResolution(width, height, pixelRatio) {
      if (hull === null || look.outline === null) return;
      hull.uniforms.resolution?.value.set(width * pixelRatio, height * pixelRatio);
      if (hull.uniforms.width !== undefined) hull.uniforms.width.value = look.outline.px * pixelRatio;
    },
    dispose() {
      for (const material of owned) material.dispose();
      hull?.dispose();
      line?.dispose();
      ramp?.dispose();
    },
  };
}

/** A glow's look material, kept in step with the glow the frame drives. */
interface Synced {
  readonly from: MeshStandardMaterial;
  readonly to: MeshStandardMaterial | MeshToonMaterial;
  readonly recolour: (from: Color, into: Color) => Color;
}

export interface Applied {
  /** Copies each glow's frame into its look material. Call after the glows update. */
  sync(): void;
  dispose(): void;
}

/** Draws `root` in `kit`'s look. `glows` are the materials a frame drives. Undo by applying the
 * `current` look's kit. */
export function applyLook(root: Object3D, kit: LookKit, glows: ReadonlySet<Material>, cache: DecorCache): Applied {
  const { look } = kit;
  const synced: Synced[] = [];
  const decor: Object3D[] = [];
  const meshes: Mesh[] = [];
  root.traverse((object) => {
    if (object instanceof Mesh && !Array.isArray(object.material) && object.userData.lookDecor !== true) meshes.push(object);
  });
  for (const mesh of meshes) {
    for (const child of [...mesh.children]) if (child.userData.lookDecor === true) mesh.remove(child);
    const source = (mesh.userData.lookSource ?? mesh.material) as Material;
    mesh.userData.lookSource = source;
    const signal = mesh.userData.ownMaterial === true;
    const material = kit.surface(source, signal);
    // A glow's material is its instance's own, so its look material is too.
    if (glows.has(source) && material !== source && source instanceof MeshStandardMaterial) {
      const to = material as MeshStandardMaterial | MeshToonMaterial;
      const recolour =
        look.surface === "pbr" ? (from: Color, into: Color) => into.copy(from) : (from: Color, into: Color) => soft(from, kit.dark, into);
      synced.push({ from: source, to, recolour });
    }
    mesh.material = material;
    mesh.castShadow = look.shadows === "map";
    mesh.receiveShadow = look.shadows === "map";
    if (look.outline === null || kit.hull === null) continue;
    const made = cache.get(mesh.geometry);
    const hull = new Mesh(made.smooth, kit.hull);
    hull.userData.lookDecor = true;
    hull.raycast = noRaycast;
    mesh.add(hull);
    decor.push(hull);
    if (kit.line !== null) {
      const lines = new LineSegments(made.edges, kit.line);
      lines.userData.lookDecor = true;
      lines.raycast = noRaycast;
      mesh.add(lines);
      decor.push(lines);
    }
  }
  return {
    sync() {
      for (const { from, to, recolour } of synced) {
        recolour(from.color, to.color);
        to.emissive.copy(from.emissive);
        to.emissiveIntensity = from.emissiveIntensity;
      }
    },
    dispose() {
      for (const object of decor) object.removeFromParent();
    },
  };
}
