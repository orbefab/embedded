// Adapted from maker-sandbox (MIT, Copyright 2026 Alwurts).
import { type ThreeEvent, useFrame, useLoader, useThree } from "@react-three/fiber";
import { Bloom, EffectComposer, Outline } from "@react-three/postprocessing";
import { type RefObject, useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { CanvasTexture, type Group, Mesh, type Object3D, RepeatWrapping, SRGBColorSpace } from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { type BuiltScene, buildScene, fromGltf, type Tag, tagOf } from "./build";
import { type CameraMode, union } from "./camera-frame";
import { CameraRig } from "./camera-rig";
import { glowIntensity, litColour } from "./glow";
import type { HoverStore } from "./hover";
import type { Command, FrameMessage, Ref, SceneMessage } from "./protocol";
import type { Box } from "./wire-geometry";
import { WireMesh } from "./wire-mesh";

const MAT_CELL_MM = 10;

function matTexture(): CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = 64;
  canvas.height = 64;
  const context = canvas.getContext("2d");
  if (context !== null) {
    context.fillStyle = "#32363c";
    context.fillRect(0, 0, 64, 64);
    context.strokeStyle = "#4a5058";
    context.strokeRect(0.5, 0.5, 63, 63);
  }
  const texture = new CanvasTexture(canvas);
  texture.wrapS = RepeatWrapping;
  texture.wrapT = RepeatWrapping;
  texture.colorSpace = SRGBColorSpace;
  return texture;
}

/** A cutting mat just under the lowest lead or wire. */
function Mat({ boxes }: { boxes: readonly Box[] }) {
  const box = useMemo(() => union(boxes), [boxes]);
  const width = Math.max(240, (box.max[0] - box.min[0]) * 2);
  const depth = Math.max(160, (box.max[2] - box.min[2]) * 2);
  const texture = useMemo(() => {
    const made = matTexture();
    made.repeat.set(width / MAT_CELL_MM, depth / MAT_CELL_MM);
    return made;
  }, [width, depth]);
  useEffect(() => () => texture.dispose(), [texture]);
  return (
    <mesh
      position={[(box.min[0] + box.max[0]) / 2, box.min[1] - 0.5, (box.min[2] + box.max[2]) / 2]}
      rotation={[-Math.PI / 2, 0, 0]}
    >
      <planeGeometry args={[width, depth]} />
      <meshStandardMaterial map={texture} roughness={1} />
    </mesh>
  );
}

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
        hiddenEdgeColor="#7dff67"
        pulseSpeed={0}
        selection={selection}
        visibleEdgeColor="#7dff67"
        xRay
      />
    </EffectComposer>
  );
}

/** The nearest object at or above `object` whose tag passes `test`. */
function nearest(object: Object3D, test: (tag: Tag) => boolean): { object: Object3D; tag: Tag } | undefined {
  for (let at: Object3D | null = object; at !== null; at = at.parent) {
    const found = tagOf(at);
    if (found !== undefined && test(found)) return { object: at, tag: found };
  }
  return undefined;
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

// Every mouse button shares one pointer, so a mouse press ends when the primary button is
// up. In a chord, the primary lifting arrives as a pointermove, not a pointerup.
function endsPress(event: PointerEvent): boolean {
  if (event.type !== "pointerup" && event.type !== "pointermove") return true;
  if (event.pointerType === "mouse") return (event.buttons & 1) === 0;
  return event.type === "pointerup";
}

export function SceneView({
  scene,
  frame,
  send,
  hover,
  mode,
  onBuilt,
}: {
  scene: SceneMessage;
  frame: RefObject<FrameMessage | null>;
  send: (command: Command) => void;
  hover: HoverStore;
  mode: CameraMode;
  onBuilt: (built: BuiltScene) => void;
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
    hover.set(
      found === undefined
        ? null
        : {
            ref: found.tag.ref,
            why: found.tag.why,
            error: found.tag.error,
            object: found.object,
            x: event.nativeEvent.clientX,
            y: event.nativeEvent.clientY,
          }
    );
  };

  return (
    <>
      <CameraRig boxes={built.boxes} mode={mode} />
      <primitive
        object={built.root}
        onPointerDown={press}
        onPointerMove={move}
        onPointerOut={() => hover.set(null)}
      />
      {built.wires.map((wire, index) => (
        <WireMesh key={index} wire={wire} />
      ))}
      <Mat boxes={built.boxes} />
      <Effects hover={hover} />
    </>
  );
}
