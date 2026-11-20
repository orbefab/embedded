// A look's stage: its lights, environment, mat, shadows and post effects. `current` draws what the
// viewer drew before looks.

import { useThree } from "@react-three/fiber";
import { Bloom, EffectComposer, N8AO, Outline } from "@react-three/postprocessing";
import { useEffect, useLayoutEffect, useMemo } from "react";
import {
  CanvasTexture,
  Color,
  type DirectionalLight,
  type Object3D,
  PCFShadowMap,
  PMREMGenerator,
  RepeatWrapping,
  ShaderMaterial,
  SRGBColorSpace,
  type Texture,
  type WebGLRenderer,
} from "three";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import { union } from "./camera-frame";
import type { Look } from "./look";
import type { Box } from "./wire-geometry";

const MAT_CELL_MM = 10;

export interface StageColours {
  readonly background: string;
  readonly mat: string;
  readonly line: string;
}

/** The colours a look paints the stage in. Most keep the page's. */
export function stageColours(look: Look, page: StageColours, dark: boolean): StageColours {
  switch (look.name) {
    case "toon":
    case "ink":
      // orbefab.com's arm: a flat paper or ink ground.
      return dark
        ? { background: "#141415", mat: "#141415", line: "#1e1e20" }
        : { background: "#f4f4f5", mat: "#f4f4f5", line: "#e2e2e5" };
    case "clay":
      return dark
        ? { background: "#24221f", mat: "#2b2925", line: "#2b2925" }
        : { background: "#ebe6de", mat: "#e4ded5", line: "#e4ded5" };
    case "studio":
      return dark
        ? { background: "#17191c", mat: "#202327", line: "#2c3036" }
        : { background: "#e9e9ec", mat: "#e2e2e6", line: "#d2d2d8" };
    default:
      return page;
  }
}

/** True when `background` is a dark colour. */
export function isDark(background: string): boolean {
  const color = new Color(background);
  return 0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b < 0.18;
}

function matTexture(fill: string, line: string): CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = 64;
  canvas.height = 64;
  const context = canvas.getContext("2d");
  if (context !== null) {
    context.fillStyle = fill;
    context.fillRect(0, 0, 64, 64);
    context.strokeStyle = line;
    context.strokeRect(0.5, 0.5, 63, 63);
  }
  const texture = new CanvasTexture(canvas);
  texture.wrapS = RepeatWrapping;
  texture.wrapT = RepeatWrapping;
  texture.colorSpace = SRGBColorSpace;
  return texture;
}

/** The cutting mat as the viewer drew it before looks: lit, with a hard edge. */
function TextureMat({ boxes, fill, line }: { boxes: readonly Box[]; fill: string; line: string }) {
  const box = useMemo(() => union(boxes), [boxes]);
  const width = Math.max(240, (box.max[0] - box.min[0]) * 2);
  const depth = Math.max(160, (box.max[2] - box.min[2]) * 2);
  const texture = useMemo(() => {
    const made = matTexture(fill, line);
    made.repeat.set(width / MAT_CELL_MM, depth / MAT_CELL_MM);
    return made;
  }, [width, depth, fill, line]);
  useEffect(() => () => texture.dispose(), [texture]);
  return (
    <mesh
      name="stage-mat"
      position={[(box.min[0] + box.max[0]) / 2, box.min[1] - 0.5, (box.min[2] + box.max[2]) / 2]}
      rotation={[-Math.PI / 2, 0, 0]}
    >
      <planeGeometry args={[width, depth]} />
      <meshStandardMaterial map={texture} roughness={1} />
    </mesh>
  );
}

const GRID_VERTEX = `
  varying vec3 vWorld;
  void main() {
    vec4 world = modelMatrix * vec4(position, 1.0);
    vWorld = world.xyz;
    gl_Position = projectionMatrix * viewMatrix * world;
  }`;

// An unlit floor with an anti-aliased grid in world mm. It fades into the background, so it has no
// edge, and it never moirés at a grazing angle as a repeated texture does.
const GRID_FRAGMENT = `
  uniform vec3 fill;
  uniform vec3 line;
  uniform vec3 major;
  uniform vec3 background;
  uniform vec2 centre;
  uniform vec2 radius;
  uniform float cell;
  uniform float lines;
  varying vec3 vWorld;
  float gridLine(vec2 p, float width) {
    vec2 g = abs(fract(p - 0.5) - 0.5) / (fwidth(p) * width);
    return 1.0 - min(min(g.x, g.y), 1.0);
  }
  void main() {
    vec2 p = vWorld.xz / cell;
    float minor = gridLine(p, 1.0) * lines;
    float big = gridLine(p / 5.0, 1.4) * lines;
    vec3 colour = mix(fill, line, minor);
    colour = mix(colour, major, big);
    float d = length((vWorld.xz - centre) / radius);
    colour = mix(colour, background, smoothstep(0.45, 1.0, d));
    gl_FragColor = vec4(colour, 1.0);
    #include <colorspace_fragment>
  }`;

function GridMat({
  boxes,
  colours,
  lines,
  shadow,
}: {
  boxes: readonly Box[];
  colours: StageColours;
  lines: boolean;
  shadow: number;
}) {
  const box = useMemo(() => union(boxes), [boxes]);
  const spanX = box.max[0] - box.min[0];
  const spanZ = box.max[2] - box.min[2];
  const radiusX = Math.max(140, spanX * 1.6);
  const radiusZ = Math.max(110, spanZ * 1.6);
  const centre: [number, number] = [(box.min[0] + box.max[0]) / 2, (box.min[2] + box.max[2]) / 2];
  const material = useMemo(
    () =>
      new ShaderMaterial({
        uniforms: {
          fill: { value: new Color() },
          line: { value: new Color() },
          major: { value: new Color() },
          background: { value: new Color() },
          centre: { value: [0, 0] },
          radius: { value: [1, 1] },
          cell: { value: MAT_CELL_MM },
          lines: { value: 1 },
        },
        vertexShader: GRID_VERTEX,
        fragmentShader: GRID_FRAGMENT,
        extensions: { derivatives: true } as never,
      }),
    []
  );
  useEffect(() => () => material.dispose(), [material]);
  useLayoutEffect(() => {
    const u = material.uniforms;
    (u.fill?.value as Color).set(colours.mat);
    (u.line?.value as Color).set(colours.line);
    (u.major?.value as Color).set(colours.line).lerp(new Color(isDark(colours.background) ? "#ffffff" : "#000000"), 0.025);
    (u.background?.value as Color).set(colours.background);
    if (u.centre !== undefined) u.centre.value = centre;
    if (u.radius !== undefined) u.radius.value = [radiusX, radiusZ];
    if (u.lines !== undefined) u.lines.value = lines ? 1 : 0;
  });
  const y = box.min[1] - 0.5;
  return (
    <>
      <mesh
        material={material}
        name="stage-mat"
        position={[centre[0], y, centre[1]]}
        renderOrder={-20}
        rotation={[-Math.PI / 2, 0, 0]}
      >
        <planeGeometry args={[radiusX * 2.2, radiusZ * 2.2]} />
      </mesh>
      {shadow > 0 ? (
        <mesh position={[centre[0], y + 0.01, centre[1]]} receiveShadow rotation={[-Math.PI / 2, 0, 0]} userData={{ treatAsOpaque: false }}>
          <planeGeometry args={[radiusX * 2.2, radiusZ * 2.2]} />
          <shadowMaterial depthWrite={false} opacity={shadow} transparent />
        </mesh>
      ) : null}
    </>
  );
}

const BLOB_VERTEX = `
  varying vec2 vUv;
  void main() {
    vUv = uv;
    gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  }`;

// orbefab.com's soft floor shadow, as a rounded rectangle: an analytic falloff with no shadow map.
const BLOB_FRAGMENT = `
  uniform float opacity;
  uniform vec2 size;
  uniform float soft;
  varying vec2 vUv;
  void main() {
    vec2 p = (vUv - 0.5) * size;
    vec2 inner = max(size * 0.5 - vec2(soft), vec2(0.0));
    vec2 q = abs(p) - inner;
    float d = length(max(q, 0.0)) / soft;
    float a = 1.0 - smoothstep(0.0, 1.0, d);
    gl_FragColor = vec4(0.0, 0.0, 0.0, a * a * opacity);
  }`;

/** A soft shadow under each part that stands on the mat. */
function BlobShadows({ boxes, opacity }: { boxes: readonly Box[]; opacity: number }) {
  const floor = useMemo(() => union(boxes).min[1], [boxes]);
  const grounded = useMemo(() => boxes.filter((box) => box.min[1] - floor < 1.5), [boxes, floor]);
  return (
    <>
      {grounded.map((box, index) => {
        // Half the short side: round under a small part, a soft-cornered slab under a board.
        const soft = Math.max(3, Math.min(box.max[0] - box.min[0], box.max[2] - box.min[2]) * 0.5);
        const width = box.max[0] - box.min[0] + soft * 2;
        const depth = box.max[2] - box.min[2] + soft * 2;
        return (
          <mesh
            key={index}
            position={[(box.min[0] + box.max[0]) / 2 + soft * 0.3, floor - 0.2, (box.min[2] + box.max[2]) / 2]}
            rotation={[-Math.PI / 2, 0, 0]}
          >
            <planeGeometry args={[width, depth]} />
            {/* Depth-tested, unlike the homepage's: a scene's parts sit over it, and a shadow drawn
                over an LED would hold it under the bloom threshold. */}
            <shaderMaterial
              depthWrite={false}
              polygonOffset
              polygonOffsetFactor={-1}
              polygonOffsetUnits={-4}
              fragmentShader={BLOB_FRAGMENT}
              transparent
              uniforms={{ opacity: { value: opacity }, size: { value: [width, depth] }, soft: { value: soft } }}
              vertexShader={BLOB_VERTEX}
            />
          </mesh>
        );
      })}
    </>
  );
}

/** The mat each look stands its parts on. */
export function LookMat({ look, boxes, colours }: { look: Look; boxes: readonly Box[]; colours: StageColours }) {
  const dark = isDark(colours.background);
  if (look.mat === "texture") return <TextureMat boxes={boxes} fill={colours.mat} line={colours.line} />;
  return (
    <>
      <GridMat
        boxes={boxes}
        colours={colours}
        lines={look.mat === "grid"}
        shadow={look.shadows === "map" ? (dark ? 0.55 : look.name === "clay" ? 0.22 : 0.28) : 0}
      />
      {look.shadows === "blob" ? <BlobShadows boxes={boxes} opacity={dark ? 0.75 : 0.32} /> : null}
    </>
  );
}

const rooms = new WeakMap<WebGLRenderer, Texture>();

/** An image-based light from three's RoomEnvironment, made once per renderer on the GPU with no
 * download. Materials take it as their own `envMap`, so each finish sets its own strength:
 * `scene.environment` would give every material one strength. */
export function roomTexture(gl: WebGLRenderer): Texture {
  let texture = rooms.get(gl);
  if (texture === undefined) {
    const pmrem = new PMREMGenerator(gl);
    const room = new RoomEnvironment();
    texture = pmrem.fromScene(room, 0.04).texture;
    room.dispose();
    pmrem.dispose();
    rooms.set(gl, texture);
  }
  return texture;
}

/** Shadow maps on while a look wants them. */
function ShadowSwitch({ on }: { on: boolean }) {
  const gl = useThree((state) => state.gl);
  useLayoutEffect(() => {
    gl.shadowMap.enabled = on;
    gl.shadowMap.type = PCFShadowMap;
    gl.shadowMap.needsUpdate = true;
  }, [gl, on]);
  return null;
}

/** A key light that casts a shadow over the whole scene. */
function ShadowKey({
  boxes,
  intensity,
  direction,
  radius,
  color = "#ffffff",
}: {
  boxes: readonly Box[];
  intensity: number;
  direction: readonly [number, number, number];
  radius: number;
  color?: string;
}) {
  const box = useMemo(() => union(boxes), [boxes]);
  const centre: [number, number, number] = [(box.min[0] + box.max[0]) / 2, box.min[1], (box.min[2] + box.max[2]) / 2];
  const reach = Math.max(box.max[0] - box.min[0], box.max[2] - box.min[2], 60) * 0.75;
  const length = Math.hypot(...direction);
  const distance = reach * 3;
  const position: [number, number, number] = [
    centre[0] + (direction[0] / length) * distance,
    centre[1] + (direction[1] / length) * distance,
    centre[2] + (direction[2] / length) * distance,
  ];
  const ref = (light: DirectionalLight | null) => {
    if (light === null) return;
    light.target.position.set(...centre);
    light.target.updateMatrixWorld();
    const camera = light.shadow.camera;
    camera.left = -reach;
    camera.right = reach;
    camera.top = reach;
    camera.bottom = -reach;
    camera.near = distance * 0.2;
    camera.far = distance * 2;
    camera.updateProjectionMatrix();
    light.shadow.mapSize.set(2048, 2048);
    light.shadow.radius = radius;
    light.shadow.bias = -0.0004;
    light.shadow.normalBias = 0.02;
  };
  return <directionalLight castShadow color={color} intensity={intensity} position={position} ref={ref} />;
}

/** Each look's lights. */
export function LookLights({ look, boxes, dark }: { look: Look; boxes: readonly Box[]; dark: boolean }) {
  switch (look.lights) {
    case "studio":
      return (
        <>
          <ShadowSwitch on />
          <hemisphereLight args={["#f2f5fa", dark ? "#1c1d20" : "#9a9ca3", 0.55]} />
          <ShadowKey boxes={boxes} direction={[-0.64, 1, 0.08]} intensity={1.9} radius={4} />
          <directionalLight intensity={0.55} position={[70, 50, -10]} />
        </>
      );
    case "clay":
      return (
        <>
          <ShadowSwitch on />
          <hemisphereLight args={["#fff8ee", dark ? "#3a342e" : "#a99c90", 1.1]} />
          <ShadowKey boxes={boxes} color="#fff4e6" direction={[-0.6, 1, 0.15]} intensity={1.7} radius={10} />
        </>
      );
    case "toon":
      // orbefab.com's arm: one flat fill and one sun, from the front left.
      return (
        <>
          <ShadowSwitch on={false} />
          <ambientLight intensity={1.05} />
          <directionalLight intensity={1.9} position={[-75, 120, 30]} />
        </>
      );
    default:
      return (
        <>
          <ShadowSwitch on={false} />
          <hemisphereLight args={["#e6ebf2", "#2a2c30", 0.9]} />
          <ambientLight intensity={0.25} />
          <directionalLight intensity={1.35} position={[40, 90, 24]} />
        </>
      );
  }
}

/** Bloom for the LEDs, the hover outline, and the look's own passes. */
export function LookEffects({ look, selection, dark }: { look: Look; selection: Object3D[]; dark: boolean }) {
  // `?hdr=1`, for tuning a look: no bloom, so a surface lit past 1, the bloom threshold, shows as white.
  // No look tone-maps: each keeps its surfaces under 1, so only an LED blooms, and the page's
  // background colour is drawn exactly.
  const hdr = typeof window !== "undefined" && new URLSearchParams(window.location.search).get("hdr") === "1";
  const ao = look.ao;
  return (
    <EffectComposer autoClear={false} multisampling={look.multisampling}>
      <>
        {ao === null ? null : (
          <N8AO
            // The pass gamma-corrects by default, as if it drew last. Bloom and the rest follow it.
            ref={(pass: { configuration: { gammaCorrection: boolean } } | null) => {
              if (pass !== null) pass.configuration.gammaCorrection = false;
            }}
            aoRadius={ao.radius}
            color="black"
            distanceFalloff={1}
            halfRes
            intensity={dark ? ao.intensity * 0.8 : ao.intensity}
            quality="medium"
          />
        )}
      </>
      <Bloom intensity={hdr ? 0 : 1.2} luminanceSmoothing={0.2} luminanceThreshold={1} mipmapBlur />
      <Outline
        blur
        edgeStrength={6}
        hiddenEdgeColor={look.hover}
        pulseSpeed={0}
        selection={selection}
        visibleEdgeColor={look.hover}
        xRay
      />
    </EffectComposer>
  );
}
