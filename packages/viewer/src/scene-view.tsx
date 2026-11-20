// Adapted from maker-sandbox (MIT, Copyright 2026 Alwurts).
import { type ThreeEvent, useFrame, useLoader, useThree } from "@react-three/fiber";
import { type RefObject, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { Box3, type Group, type Material, Mesh, type Object3D } from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { type BuiltScene, buildScene, fromGltf, type Tag, tagOf } from "./build";
import type { CameraMode } from "./camera-frame";
import { CameraRig } from "./camera-rig";
import { glowIntensity, litColour } from "./glow";
import type { Hovered, HoverStore } from "./hover";
import type { Look } from "./look";
import { type Applied, applyLook, createKit, DecorCache } from "./look-materials";
import { isDark, LookEffects, LookLights, LookMat, roomTexture, type StageColours } from "./look-stage";
import type { Command, FrameMessage, Ref, SceneMessage, Vec3 } from "./protocol";
import { holeAt, instanceAt, refOf, stripText } from "./refs";
import type { Box } from "./wire-geometry";
import { WireMesh } from "./wire-mesh";

/** The meshes under the hovered object, for its outline. */
function useSelection(hover: HoverStore): Object3D[] {
  const hovered = useSyncExternalStore(hover.subscribe, hover.get);
  const target = hovered?.object;
  return useMemo(() => {
    const meshes: Object3D[] = [];
    target?.traverse((object) => {
      if (object instanceof Mesh && object.userData.lookDecor !== true) meshes.push(object);
    });
    return meshes;
  }, [target]);
}

/** One box per drawn mesh, and one per wire, in the viewer's world. */
function drawnBoxes(built: BuiltScene): Box[] {
  const boxes: Box[] = [];
  const box = new Box3();
  built.root.updateMatrixWorld(true);
  built.root.traverse((object) => {
    if (!(object instanceof Mesh) || object.userData.lookDecor === true) return;
    object.geometry.boundingBox ?? object.geometry.computeBoundingBox();
    if (object.geometry.boundingBox === null) return;
    box.copy(object.geometry.boundingBox).applyMatrix4(object.matrixWorld);
    boxes.push({ min: box.min.toArray(), max: box.max.toArray() });
  });
  return [...boxes, ...built.boxes.slice(built.boxes.length - built.wires.length)];
}

/** The nearest object at or above `object` whose tag passes `test`. */
function nearest(object: Object3D, test: (tag: Tag) => boolean): { object: Object3D; tag: Tag } | undefined {
  for (let at: Object3D | null = object; at !== null; at = at.parent) {
    const found = tagOf(at);
    if (found !== undefined && test(found)) return { object: at, tag: found };
  }
  return undefined;
}

/** A breadboard's hole under the pointer, as its ref and the strip it sits in. */
function holeUnder(scene: SceneMessage, ref: Ref, group: Object3D, event: ThreeEvent<PointerEvent>): Hovered | undefined {
  const path = ref.slice(1).split(".");
  const placed = instanceAt(scene, path);
  if (placed?.part.strips === undefined) return undefined;
  const local = group.worldToLocal(event.point.clone()).toArray() as Vec3;
  const hole = holeAt(placed.part, local);
  if (hole === undefined) return undefined;
  const board = refOf(path).slice(1);
  return {
    ref: refOf([...path, hole.pin]),
    why: undefined,
    error: undefined,
    strip: `strip ${stripText(hole.strip, `${board}.`)}`,
    object: group,
    x: event.nativeEvent.clientX,
    y: event.nativeEvent.clientY,
  };
}

function modelUrls(scene: SceneMessage): string[] {
  const urls = new Set<string>();
  for (const part of Object.values(scene.parts)) if (part.model !== null) urls.add(part.model);
  return [...urls];
}

/** The press a pointer holds, and how to stop listening for its end. */
interface Held {
  readonly ref: Ref;
  readonly pointerId: number;
  readonly stop: () => void;
}

const PRESS_ENDS = ["pointerup", "pointermove", "pointercancel", "lostpointercapture"] as const;

// A pointer's buttons share it, so a press ends when the primary (a mouse's left button,
// a pen's tip, a finger's contact) is up. In a chord, the primary lifting arrives as a
// pointermove, not a pointerup.
function endsPress(event: PointerEvent): boolean {
  if (event.type !== "pointerup" && event.type !== "pointermove") return true;
  return (event.buttons & 1) === 0;
}

export function SceneView({
  scene,
  frame,
  send,
  hover,
  mode,
  onBuilt,
  look,
  colours,
  pageScroll = false,
}: {
  scene: SceneMessage;
  frame: RefObject<FrameMessage | null>;
  send: (command: Command) => void;
  hover: HoverStore;
  mode: CameraMode;
  onBuilt: (built: BuiltScene) => void;
  look: Look;
  /** The stage's background, mat fill and grid line, in the look's colours. */
  colours: StageColours;
  /** One finger scrolls the page. Two fingers orbit and zoom. */
  pageScroll?: boolean;
}) {
  const urls = useMemo(() => modelUrls(scene), [scene]);
  const gltfs = useLoader(GLTFLoader, urls);
  const built = useMemo(() => {
    const models = new Map<string, Group | Error>(
      urls.map((url, index) => {
        const gltf = gltfs[index];
        if (gltf === undefined) return [url, new Error(`${url}: not loaded`)];
        try {
          return [url, fromGltf(gltf, url)];
        } catch (error) {
          return [url, error instanceof Error ? error : new Error(String(error))];
        }
      })
    );
    return buildScene(scene, models);
  }, [gltfs, scene, urls]);
  useEffect(() => onBuilt(built), [built, onBuilt]);

  const dark = isDark(colours.background);
  const gl = useThree((state) => state.gl);
  const kit = useMemo(() => createKit(look, dark, look.environment > 0 ? roomTexture(gl) : null), [look, dark, gl]);
  useEffect(() => () => kit.dispose(), [kit]);
  const decor = useMemo(() => new DecorCache(), [built]);
  useEffect(() => () => decor.dispose(), [decor]);
  const applied = useRef<Applied | null>(null);
  useLayoutEffect(() => {
    const glows = new Set<Material>(built.glows.map((glow) => glow.material));
    const next = applyLook(built.root, kit, glows, decor);
    applied.current = next;
    return () => {
      next.dispose();
      if (applied.current === next) applied.current = null;
    };
  }, [built, kit, decor]);

  useFrame((state) => {
    const live = frame.current;
    for (const glow of built.glows) {
      const intensity = glowIntensity(live?.led_a[glow.channel] ?? 0, glow.full);
      glow.material.emissiveIntensity = intensity;
      litColour(glow.base, intensity, glow.material.color);
    }
    applied.current?.sync();
    kit.setResolution(state.size.width, state.size.height, state.viewport.dpr);
    for (const slide of built.slides) {
      slide.node.position.copy(slide.rest);
      if (live?.switches[slide.channel] === 1) slide.node.position.add(slide.delta);
    }
  });

  const canvas = useThree((state) => state.gl.domElement);
  const held = useRef<Held | null>(null);
  const release = useCallback(() => {
    const pressed = held.current;
    if (pressed === null) return;
    held.current = null;
    pressed.stop();
    send({ cmd: "release", ref: pressed.ref });
  }, [send]);
  useEffect(() => {
    window.addEventListener("blur", release);
    return () => {
      window.removeEventListener("blur", release);
      release();
    };
  }, [release]);
  // A new scene follows an `open`, which restarts the session, so a press held across it
  // is dropped without a release.
  useEffect(
    () => () => {
      held.current?.stop();
      held.current = null;
    },
    [scene]
  );

  // React Three Fiber never calls an object's onPointerCancel or onLostPointerCapture, so
  // the press listens for its own pointer's end on the canvas.
  const press = (event: ThreeEvent<PointerEvent>) => {
    event.stopPropagation();
    if (event.button !== 0) return;
    const instance = nearest(event.object, (tag) => tag.instance !== undefined);
    if (instance?.tag.instance?.pressable !== true || held.current !== null) return;
    const { ref } = instance.tag;
    const { pointerId } = event;
    const end = (native: PointerEvent) => {
      if (native.pointerId === held.current?.pointerId && endsPress(native)) release();
    };
    for (const kind of PRESS_ENDS) canvas.addEventListener(kind, end);
    held.current = {
      ref,
      pointerId,
      stop: () => {
        for (const kind of PRESS_ENDS) canvas.removeEventListener(kind, end);
      },
    };
    (event.target as Element).setPointerCapture(pointerId);
    send({ cmd: "press", ref });
  };
  const move = (event: ThreeEvent<PointerEvent>) => {
    event.stopPropagation();
    const found = nearest(event.object, () => true);
    if (found === undefined) {
      hover.set(null);
      return;
    }
    const at = { object: found.object, x: event.nativeEvent.clientX, y: event.nativeEvent.clientY };
    const hole = found.tag.instance === undefined ? undefined : holeUnder(scene, found.tag.ref, found.object, event);
    hover.set(hole ?? { ref: found.tag.ref, why: found.tag.why, error: found.tag.error, ...at });
  };

  const selection = useSelection(hover);
  // `current` frames each part's box. The other looks frame what is drawn: each mesh's own box,
  // so a tall part's empty corners do not push the camera back.
  const frameBoxes = useMemo(() => (look.name === "current" ? built.boxes : drawnBoxes(built)), [built, look.name]);
  return (
    <>
      <CameraRig boxes={frameBoxes} mode={mode} pageScroll={pageScroll} style={look.camera} />
      <LookLights boxes={built.boxes} dark={dark} look={look} />
      <primitive
        object={built.root}
        onPointerDown={press}
        onPointerMove={move}
        onPointerOut={() => hover.set(null)}
      />
      {built.wires.map((wire, index) => (
        <WireMesh key={index} kit={kit} wire={wire} />
      ))}
      <LookMat boxes={built.boxes} colours={colours} look={look} />
      <LookEffects dark={dark} look={look} selection={selection} />
    </>
  );
}
