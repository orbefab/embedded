import { type ThreeEvent, useFrame, useLoader } from "@react-three/fiber";
import { Bloom, EffectComposer, Outline } from "@react-three/postprocessing";
import { type RefObject, useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { CanvasTexture, Mesh, type Object3D, RepeatWrapping, SRGBColorSpace } from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { buildScene, fromGltf, type Tag, tagOf } from "./build";
import { type CameraMode, union } from "./camera-frame";
import { CameraRig } from "./camera-rig";
import { glowIntensity } from "./glow";
import type { HoverStore } from "./hover";
import type { FrameMessage, Ref, SceneMessage, SessionSource } from "./protocol";
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

export function SceneView({
  scene,
  frame,
  source,
  hover,
  mode,
}: {
  scene: SceneMessage;
  frame: RefObject<FrameMessage | null>;
  source: SessionSource;
  hover: HoverStore;
  mode: CameraMode;
}) {
  const urls = useMemo(() => modelUrls(scene), [scene]);
  const gltfs = useLoader(GLTFLoader, urls);
  const built = useMemo(() => {
    const models = new Map(
      urls.map((url, index) => {
        const gltf = gltfs[index];
        if (gltf === undefined) throw new Error(`${url}: not loaded`);
        return [url, fromGltf(gltf, url)] as const;
      })
    );
    return buildScene(scene, models);
  }, [gltfs, scene, urls]);

  useFrame(() => {
    const live = frame.current;
    for (const glow of built.glows) {
      glow.material.emissiveIntensity = glowIntensity(live?.led_a[glow.channel] ?? 0, glow.full);
    }
    for (const slide of built.slides) {
      slide.node.position.copy(slide.rest);
      if (live?.switches[slide.channel] === 1) slide.node.position.add(slide.delta);
    }
  });

  const held = useRef<Ref | null>(null);
  const release = useCallback(() => {
    const ref = held.current;
    if (ref === null) return;
    held.current = null;
    source.send({ cmd: "release", ref });
  }, [source]);
  useEffect(() => {
    window.addEventListener("blur", release);
    return () => {
      window.removeEventListener("blur", release);
      release();
    };
  }, [release]);

  const press = (event: ThreeEvent<PointerEvent>) => {
    event.stopPropagation();
    const instance = nearest(event.object, (tag) => tag.instance !== undefined);
    if (instance?.tag.instance?.pressable !== true || held.current !== null) return;
    held.current = instance.tag.ref;
    (event.target as Element).setPointerCapture(event.pointerId);
    source.send({ cmd: "press", ref: instance.tag.ref });
  };
  const move = (event: ThreeEvent<PointerEvent>) => {
    event.stopPropagation();
    const found = nearest(event.object, () => true);
    hover.set(
      found === undefined
        ? null
        : { ref: found.tag.ref, object: found.object, x: event.nativeEvent.clientX, y: event.nativeEvent.clientY }
    );
  };

  return (
    <>
      <CameraRig boxes={built.boxes} mode={mode} />
      <primitive
        object={built.root}
        onLostPointerCapture={release}
        onPointerCancel={release}
        onPointerDown={press}
        onPointerMove={move}
        onPointerOut={() => hover.set(null)}
        onPointerUp={release}
      />
      {built.wires.map((wire, index) => (
        <WireMesh key={index} wire={wire} />
      ))}
      <Mat boxes={built.boxes} />
      <Effects hover={hover} />
    </>
  );
}
