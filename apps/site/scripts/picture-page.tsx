// The page apps/site/scripts/pictures.ts drives in headless Chromium. It draws one part's view.json,
// or one Block's scene as the Block runs, with the part page's look, the viewer's room light, key
// light and materials, on a clear background with no floor, from the part page's camera angle
// framed tight on what it draws, and returns the picture as WebP. `window.drawPart(viewUrl)`
// draws a part. `window.drawBlock(id, atMs, press, limitMs)` runs a Block in orbe-wasm's Worker as
// the Block page's player does, holding `press` down from the start when it names a part, pauses
// it on the first frame at or past `atMs` of simulated time, and draws the scene as that frame
// shows it.
import { type FrameMessage, loadBlock, type Ref, type Reply, type SceneMessage, type ScreenImage, workerSource } from "@orbe/viewer";
import { buildScene, type DrawnWire, fromGltf } from "@orbe/viewer/build";
import { cameraPosition } from "@orbe/viewer/camera-frame";
import { poseFrame } from "@orbe/viewer/pose";
import { applyRoom, roomTexture, StageLights, StageRoom } from "@orbe/viewer/stage";
import { WireMesh } from "@orbe/viewer/wire-mesh";
import { Canvas, useThree } from "@react-three/fiber";
import { useLayoutEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { BackSide, Box3, type Group, Mesh, MeshStandardMaterial, type Object3D, PerspectiveCamera, Quaternion, Vector3 } from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { at } from "../src/paths.ts";

/** The picture's size in pixels. The canvas draws at twice that and is scaled down once. */
export const WIDTH = 480;
export const HEIGHT = 360;
const SCALE = 2;
/** The part fills the frame up to this fraction of its half width and half height. */
const FILL = 0.88;
const QUALITY = 0.86;
/** A light page's stage colour. The hemisphere light's ground colour depends on it. */
const LIGHT_STAGE = "#e9e9ec";
const FOV_DEG = 30;

interface Drawn {
  readonly root: Group;
  readonly wires: readonly DrawnWire[];
  readonly boxes: Parameters<typeof StageLights>[0]["boxes"];
  readonly done: (picture: string) => void;
}

/** What `drawBlock` returns: the picture, and the simulated time of the frame it shows. */
interface BlockPicture {
  readonly picture: string;
  readonly t_ps: number;
}

const loader = new GLTFLoader();
const models = new Map<string, Promise<Group | Error>>();

/** A part's model. A Block's models are object URLs its Worker source frees on close, so they are
 * loaded fresh each time and never kept. */
function model(url: string): Promise<Group | Error> {
  let loaded = models.get(url);
  if (loaded === undefined) {
    loaded = loader.loadAsync(url).then(
      (gltf) => fromGltf(gltf, url),
      (error: unknown) => (error instanceof Error ? error : new Error(String(error))),
    );
    if (!url.startsWith("blob:")) models.set(url, loaded);
  }
  return loaded;
}

/** The scene's parts built from their models, or an Error naming what did not build. */
async function build(scene: SceneMessage) {
  const urls = [...new Set(Object.values(scene.parts).flatMap((part) => (part.model === null ? [] : [part.model])))];
  const loaded = new Map(await Promise.all(urls.map(async (each) => [each, await model(each)] as const)));
  const built = buildScene(scene, loaded);
  if (built.errors.length > 0) throw new Error(built.errors.join("\n"));
  return built;
}

/** Every vertex the part draws, in the viewer's world. */
function points(root: Object3D): Vector3[] {
  const out: Vector3[] = [];
  root.updateMatrixWorld(true);
  root.traverse((object) => {
    if (!(object instanceof Mesh) || !object.visible) return;
    const position = object.geometry.getAttribute("position");
    if (position === undefined) return;
    const step = Math.max(1, Math.floor(position.count / 4000));
    for (let index = 0; index < position.count; index += step) {
      out.push(new Vector3().fromBufferAttribute(position, index).applyMatrix4(object.matrixWorld));
    }
  });
  return out;
}

/** Puts the camera on the part page's tilted direction and moves it until the part's
 * outline sits centred and fills {@link FILL} of the frame. */
function frame(camera: PerspectiveCamera, root: Object3D): void {
  const all = points(root);
  const box = new Box3().setFromPoints(all);
  const target = box.getCenter(new Vector3());
  const back = new Vector3(...cameraPosition({ mode: "tilted", target: [0, 0, 0], distance: 1 })).normalize();
  const span = box.getSize(new Vector3()).length() || 1;
  let distance = span * 3;
  const tanV = Math.tan((FOV_DEG * Math.PI) / 360);
  const tanH = tanV * (WIDTH / HEIGHT);
  const ndc = new Vector3();
  for (let pass = 0; pass < 12; pass += 1) {
    camera.position.copy(target).addScaledVector(back, distance);
    camera.lookAt(target);
    camera.near = Math.max(0.1, distance / 200);
    camera.far = distance * 20;
    camera.updateProjectionMatrix();
    camera.updateMatrixWorld(true);
    let [minX, maxX, minY, maxY] = [Infinity, -Infinity, Infinity, -Infinity];
    for (const point of all) {
      ndc.copy(point).project(camera);
      minX = Math.min(minX, ndc.x);
      maxX = Math.max(maxX, ndc.x);
      minY = Math.min(minY, ndc.y);
      maxY = Math.max(maxY, ndc.y);
    }
    // Move the target across the view by the outline's offset, at the target's depth.
    const right = new Vector3().setFromMatrixColumn(camera.matrixWorld, 0);
    const up = new Vector3().setFromMatrixColumn(camera.matrixWorld, 1);
    target.addScaledVector(right, ((minX + maxX) / 2) * distance * tanH);
    target.addScaledVector(up, ((minY + maxY) / 2) * distance * tanV);
    const fill = Math.max((maxX - minX) / 2, (maxY - minY) / 2) / FILL;
    distance *= 0.5 + 0.5 * fill;
  }
  camera.position.copy(target).addScaledVector(back, distance);
  camera.lookAt(target);
  camera.updateMatrixWorld(true);
}

/** Lights the part or the Block as the part page does, frames it, waits for two frames and reads
 * the canvas. */
function Picture({ drawn }: { drawn: Drawn }) {
  const gl = useThree((state) => state.gl);
  const camera = useThree((state) => state.camera) as PerspectiveCamera;
  const invalidate = useThree((state) => state.invalidate);
  const all = useRef<Group>(null);
  useLayoutEffect(() => {
    const texture = roomTexture(gl);
    const content = all.current;
    if (content === null) throw new Error("nothing to draw");
    content.traverse((object) => {
      if (!(object instanceof Mesh)) return;
      object.castShadow = true;
      object.receiveShadow = true;
      for (const material of Array.isArray(object.material) ? object.material : [object.material]) {
        if (!(material instanceof MeshStandardMaterial)) continue;
        material.shadowSide = BackSide;
        applyRoom(material, texture);
      }
    });
    frame(camera, content);
    let frames = 0;
    let handle = 0;
    const wait = () => {
      invalidate();
      frames += 1;
      if (frames < 4) {
        handle = requestAnimationFrame(wait);
        return;
      }
      const out = document.createElement("canvas");
      out.width = WIDTH;
      out.height = HEIGHT;
      const context = out.getContext("2d");
      if (context === null) throw new Error("no 2d canvas");
      context.imageSmoothingQuality = "high";
      context.drawImage(gl.domElement, 0, 0, WIDTH, HEIGHT);
      drawn.done(out.toDataURL("image/webp", QUALITY));
    };
    handle = requestAnimationFrame(wait);
    return () => cancelAnimationFrame(handle);
  }, [camera, drawn, gl, invalidate]);
  return (
    <>
      <StageRoom />
      <StageLights background={LIGHT_STAGE} boxes={drawn.boxes} />
      <group ref={all}>
        <primitive object={drawn.root} />
        {drawn.wires.map((wire, index) => (
          <WireMesh key={index} wire={wire} />
        ))}
      </group>
    </>
  );
}

function App() {
  const [drawn, setDrawn] = useState<Drawn | null>(null);
  useLayoutEffect(() => {
    const turned = new Quaternion();
    Object.assign(window, {
      async drawPart(url: string): Promise<string> {
        const response = await fetch(url);
        if (!response.ok) throw new Error(`${url}: ${response.status}`);
        const built = await build((await response.json()) as SceneMessage);
        // A knob or a servo horn sits where the part page draws it before any frame.
        for (const turn of built.turns) {
          turned.setFromAxisAngle(turn.axis, turn.angle(turn.value));
          turn.node.quaternion.copy(turned).multiply(turn.rest);
        }
        return new Promise<string>((resolve) => setDrawn({ root: built.root, wires: [], boxes: built.boxes, done: resolve }));
      },

      /** Runs Block `id` until `atMs` of simulated time, or rejects once `limitMs` of wall time
       * passed first, or the Block did not open, or its session faulted or stopped. */
      drawBlock(id: string, atMs: number, press: Ref | null, limitMs: number): Promise<BlockPicture> {
        const source = workerSource(
          () => new Worker(new URL(`${at.wasm}worker.js`, document.baseURI), { type: "module" }),
          () => loadBlock(new URL(at.index, document.baseURI), id),
        );
        const atPs = atMs * 1e9;
        return new Promise<BlockPicture>((resolve, reject) => {
          let scene: SceneMessage | null = null;
          let last: FrameMessage | null = null;
          let done = false;
          // Each screen's last glass, as the viewer keeps it: a frame carries only what changed.
          const screens: (ScreenImage | undefined)[] = [];
          let timer = 0;
          let off = () => {};
          const finish = (outcome: () => void) => {
            if (done) return;
            done = true;
            window.clearTimeout(timer);
            off();
            outcome();
          };
          const fail = (why: string) => finish(() => {
            source.close();
            reject(new Error(why));
          });
          timer = window.setTimeout(() => {
            const reached = last === null ? "no frame" : `t = ${(last.t_ps / 1e12).toFixed(3)} s`;
            fail(`reached ${reached} of the ${atMs} ms the picture shows in ${limitMs / 1000} s of wall time`);
          }, limitMs);
          off = source.subscribe((message) => {
            if (message.type === "closed") return fail(`the session ended: ${message.reason}`);
            if (message.type === "scene") {
              scene = message;
              const refused = (what: string) => (reply: Reply) => {
                if (!reply.ok) fail(`${what}: ${reply.error.message}`);
              };
              const lost = (error: unknown) => fail(String(error));
              if (press !== null) source.send({ cmd: "press", ref: press }).then(refused(`press ${press}`), lost);
              source.send({ cmd: "play" }).then(refused("play"), lost);
              return;
            }
            last = message;
            for (const [channel, image] of (message.screens ?? []).entries()) if (image !== null) screens[channel] = image;
            if (message.state === "faulted" || message.state === "stopped") return fail(`the session ${message.state}: ${message.fault ?? ""}`);
            if (message.t_ps < atPs || scene === null) return;
            const shown = scene;
            finish(() => {
              void source.send({ cmd: "pause" }).catch(() => {});
              build(shown).then(
                (built) => {
                  // The LEDs glow at the mean current of the frame shown, with no earlier peak.
                  poseFrame(built, message, [], screens);
                  setDrawn({
                    root: built.root,
                    wires: built.wires,
                    boxes: built.boxes,
                    done: (picture) => {
                      source.close();
                      resolve({ picture, t_ps: message.t_ps });
                    },
                  });
                },
                (error: unknown) => {
                  source.close();
                  reject(error instanceof Error ? error : new Error(String(error)));
                },
              );
            });
          });
        });
      },
    });
  }, []);
  return (
    <div style={{ width: WIDTH, height: HEIGHT }}>
      <Canvas
        camera={new PerspectiveCamera(FOV_DEG, WIDTH / HEIGHT, 0.1, 4000)}
        dpr={SCALE}
        flat
        gl={{ alpha: true, antialias: true, preserveDrawingBuffer: true }}
        onCreated={({ gl }) => gl.setClearColor(0x000000, 0)}
      >
        {drawn === null ? null : <Picture drawn={drawn} key={drawn.root.uuid} />}
      </Canvas>
    </div>
  );
}

const mount = document.getElementById("root");
if (mount === null) throw new Error("no #root");
createRoot(mount).render(<App />);
