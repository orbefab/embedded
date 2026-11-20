// Adapted from maker-sandbox (MIT, Copyright 2026 Alwurts).
import { OrbitControls } from "@react-three/drei/core/OrbitControls";
import { OrthographicCamera } from "@react-three/drei/core/OrthographicCamera";
import { PerspectiveCamera } from "@react-three/drei/core/PerspectiveCamera";
import { useThree } from "@react-three/fiber";
import { useEffect } from "react";
import { MOUSE, OrthographicCamera as OrthoImpl, TOUCH } from "three";
import type { OrbitControls as OrbitControlsImpl } from "three/addons/controls/OrbitControls.js";
import {
  type CameraMode,
  type CameraStyle,
  cameraPosition,
  DEFAULT_CAMERA,
  frameView,
  MIN_DISTANCE_MM,
  TOP_VIEW_ZOOM,
  tiltedOffset,
} from "./camera-frame";
import type { Box } from "./wire-geometry";

const ORBIT_MOUSE = { LEFT: MOUSE.ROTATE, MIDDLE: MOUSE.DOLLY, RIGHT: MOUSE.PAN };
const ORBIT_TOUCH = { ONE: TOUCH.ROTATE, TWO: TOUCH.DOLLY_PAN };
const TOP_MOUSE = { LEFT: MOUSE.PAN, MIDDLE: MOUSE.DOLLY, RIGHT: MOUSE.PAN };
const TOP_TOUCH = { ONE: TOUCH.PAN, TWO: TOUCH.DOLLY_PAN };
// One finger is not a control. A value outside the TOUCH enum hits OrbitControls' default and
// leaves that finger alone, so the page can scroll it.
const PAGE_FINGER = -1 as TOUCH;
const PAGE_ORBIT_TOUCH = { ONE: PAGE_FINGER, TWO: TOUCH.DOLLY_ROTATE };
const PAGE_TOP_TOUCH = { ONE: PAGE_FINGER, TWO: TOUCH.DOLLY_PAN };

/** OrbitControls sets `touch-action: none` on its own element. That property is not inherited, so
 * the canvas underneath would still take the finger. Page mode sets `pan-y` on both. One finger
 * is left out of the TOUCH enum, so the controls ignore it and the page can scroll. */
function pageTouches(element: HTMLElement): () => void {
  const canvases = element instanceof HTMLCanvasElement ? [element] : [...element.querySelectorAll("canvas")];
  const paint = () => {
    element.style.touchAction = "pan-y";
    for (const canvas of canvases) canvas.style.touchAction = "pan-y";
  };
  paint();
  return () => {
    element.style.touchAction = "none";
    for (const canvas of canvases) canvas.style.touchAction = "none";
  };
}
// The top camera's up is world -Z, so the part frame's +Y points up the screen. The camera
// has it from its creation, so the controls orbit about Z, and they hold the camera on +Y
// above the target: polar angle 90 degrees, azimuth 0.
const TOP_UP = [0, 0, -1] as const;

/** Orbit in 3D, or pan and zoom straight down in the top view. Each mode, and each new
 * scene, starts framed on `boxes`. With `pageScroll`, one finger scrolls the page and two
 * fingers orbit and zoom (or pan and zoom, in the top view). */
export function CameraRig({
  mode,
  boxes,
  pageScroll = false,
  style = DEFAULT_CAMERA,
}: {
  mode: CameraMode;
  boxes: readonly Box[];
  pageScroll?: boolean;
  style?: CameraStyle;
}) {
  const camera = useThree((state) => state.camera);
  const controls = useThree((state) => state.controls) as OrbitControlsImpl | null;
  const get = useThree((state) => state.get);
  const [ox, oy, oz] = tiltedOffset(style);

  useEffect(() => {
    if (controls === null) return;
    const { size } = get();
    const view = frameView(mode, boxes, size, style);
    const [px, py, pz] = cameraPosition(view, style);
    camera.position.set(px, py, pz);
    if (camera instanceof OrthoImpl && view.mode === "top") camera.zoom = view.zoom;
    camera.updateProjectionMatrix();
    if (view.mode === "tilted") controls.maxDistance = view.distance * 4;
    controls.target.set(...view.target);
    controls.update();
  }, [boxes, camera, controls, get, mode, style]);

  useEffect(() => {
    if (!pageScroll || controls === null || !(controls.domElement instanceof HTMLElement)) return;
    return pageTouches(controls.domElement);
  }, [controls, pageScroll]);

  return (
    <>
      {mode === "top" ? (
        <OrthographicCamera
          far={2000}
          makeDefault
          near={0.1}
          position={[0, 400, 0]}
          up={TOP_UP}
          zoom={TOP_VIEW_ZOOM}
        />
      ) : (
        <PerspectiveCamera far={4000} fov={style.fovDeg} makeDefault near={0.1} position={[ox, oy, oz]} />
      )}
      <OrbitControls
        dampingFactor={0.12}
        enableDamping
        enableRotate={mode === "tilted"}
        keyEvents={false}
        makeDefault
        maxAzimuthAngle={mode === "tilted" ? Number.POSITIVE_INFINITY : 0}
        maxPolarAngle={mode === "tilted" ? Math.PI / 2.2 : Math.PI / 2}
        minAzimuthAngle={mode === "tilted" ? Number.NEGATIVE_INFINITY : 0}
        minDistance={MIN_DISTANCE_MM}
        minPolarAngle={mode === "tilted" ? 0.08 : Math.PI / 2}
        mouseButtons={mode === "top" ? TOP_MOUSE : ORBIT_MOUSE}
        touches={pageScroll ? (mode === "top" ? PAGE_TOP_TOUCH : PAGE_ORBIT_TOUCH) : mode === "top" ? TOP_TOUCH : ORBIT_TOUCH}
      />
    </>
  );
}
