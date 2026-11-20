// Adapted from maker-sandbox (MIT, Copyright 2026 Alwurts).
import { type ThreeEvent, useFrame, useLoader, useThree } from "@react-three/fiber";
import { Bloom, EffectComposer, Outline } from "@react-three/postprocessing";
import { type RefObject, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { BackSide, Box3, type Group, Mesh, MeshStandardMaterial, type Object3D } from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { type BuiltScene, buildScene, fromGltf, type Tag, tagOf } from "./build";
import type { CameraMode } from "./camera-frame";
import { CameraRig } from "./camera-rig";
import { glowIntensity, litColour } from "./glow";
import type { Hovered, HoverStore } from "./hover";
import type { Command, FrameMessage, Ref, SceneMessage, Vec3 } from "./protocol";
import { holeAt, instanceAt, refOf, stripText } from "./refs";
import { applyPlasticRoom, roomTexture, StageFloor, StageLights, StageRoom } from "./stage";
import type { StageTheme } from "./stage-theme";
import type { Box } from "./wire-geometry";
import { WireMesh } from "./wire-mesh";

/** orbefab.com's brand orange. */
const HOVER = "#e85d04";

function Effects({ hover }: { hover: HoverStore }) {
  const hovered = useSyncExternalStore(hover.subscribe, hover.get);
  const target = hovered?.object;
  const selection = useMemo(() => {
    const meshes: Object3D[] = [];
    target?.traverse((object) => {
      if (object instanceof Mesh) meshes.push(object);
    });
    return meshes;
  }, [target]);
  return (
    <EffectComposer autoClear={false} multisampling={0}>
      <Bloom intensity={1.2} luminanceSmoothing={0.2} luminanceThreshold={1} mipmapBlur />
      <Outline
        blur
        edgeStrength={6}
        hiddenEdgeColor={HOVER}
        pulseSpeed={0}
        selection={selection}
        visibleEdgeColor={HOVER}
        xRay
      />
    </EffectComposer>
  );
}

/** One box per drawn mesh, plus one per wire. A part's own box has empty corners, and those
 * would push the camera back. */
function drawnBoxes(built: BuiltScene): readonly Box[] {
  const boxes: Box[] = [];
  const box = new Box3();
  built.root.updateMatrixWorld(true);
  built.root.traverse((object) => {
    if (!(object instanceof Mesh)) return;
    object.geometry.boundingBox ?? object.geometry.computeBoundingBox();
    if (object.geometry.boundingBox === null) return;
    box.copy(object.geometry.boundingBox).applyMatrix4(object.matrixWorld);
    boxes.push({ min: [box.min.x, box.min.y, box.min.z], max: [box.max.x, box.max.y, box.max.z] });
  });
  if (boxes.length === 0) return built.boxes;
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
  colours,
  pageScroll = false,
}: {
  scene: SceneMessage;
  frame: RefObject<FrameMessage | null>;
  send: (command: Command) => void;
  hover: HoverStore;
  mode: CameraMode;
  onBuilt: (built: BuiltScene) => void;
  /** Background, floor fill and grid line, from the page theme. */
  colours: StageTheme;
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

  const gl = useThree((state) => state.gl);
  const stage = useThree((state) => state.scene);
  useLayoutEffect(() => {
    const texture = roomTexture(gl);
    const visit = (object: Object3D) => {
      if (!(object instanceof Mesh)) return;
      object.castShadow = true;
      object.receiveShadow = true;
      const materials = Array.isArray(object.material) ? object.material : [object.material];
      for (const material of materials) {
        if (!(material instanceof MeshStandardMaterial)) continue;
        // A pin or a shell is open. The back faces cast, so the shadow keeps the part's shape.
        material.shadowSide = BackSide;
        applyPlasticRoom(material, texture);
      }
    };
    built.root.traverse(visit);
    stage.getObjectByName("stage-wires")?.traverse(visit);
  }, [built, gl, stage]);

  useFrame(() => {
    const live = frame.current;
    for (const glow of built.glows) {
      const intensity = glowIntensity(live?.led_a[glow.channel] ?? 0, glow.full);
      glow.material.emissiveIntensity = intensity;
      litColour(glow.base, intensity, glow.material.color);
    }
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

  const frameBoxes = useMemo(() => drawnBoxes(built), [built]);
  return (
    <>
      <StageRoom />
      <StageLights background={colours.background} boxes={built.boxes} />
      <CameraRig boxes={frameBoxes} mode={mode} pageScroll={pageScroll} />
      <primitive
        object={built.root}
        onPointerDown={press}
        onPointerMove={move}
        onPointerOut={() => hover.set(null)}
      />
      <group name="stage-wires">
        {built.wires.map((wire, index) => (
          <WireMesh key={index} wire={wire} />
        ))}
      </group>
      <StageFloor boxes={built.boxes} colours={colours} />
      <Effects hover={hover} />
    </>
  );
}
