// Adapted from maker-sandbox (MIT, Copyright 2026 Alwurts).
import { OrbitControls } from "@react-three/drei/core/OrbitControls";
import { OrthographicCamera } from "@react-three/drei/core/OrthographicCamera";
import { PerspectiveCamera } from "@react-three/drei/core/PerspectiveCamera";
import { useThree } from "@react-three/fiber";
import { useEffect } from "react";
import { MOUSE, OrthographicCamera as OrthoImpl, TOUCH } from "three";
import type { OrbitControls as OrbitControlsImpl } from "three/addons/controls/OrbitControls.js";
import {
  CAMERA_FOV_DEG,
  type CameraMode,
  cameraPosition,
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
// The top camera's up is world -Z, so the part frame's +Y points up the screen. The camera
// has it from its creation, so the controls orbit about Z, and they hold the camera on +Y
// above the target: polar angle 90 degrees, azimuth 0.
const TOP_UP = [0, 0, -1] as const;

/** Orbit in 3D, or pan and zoom straight down in the top view. Each mode, and each new
 * scene, starts framed on `boxes`. */
export function CameraRig({ mode, boxes }: { mode: CameraMode; boxes: readonly Box[] }) {
  const camera = useThree((state) => state.camera);
  const controls = useThree((state) => state.controls) as OrbitControlsImpl | null;
  const get = useThree((state) => state.get);
  const [ox, oy, oz] = tiltedOffset();

  useEffect(() => {
    if (controls === null) return;
    const { size } = get();
    const view = frameView(mode, boxes, size);
    const [px, py, pz] = cameraPosition(view);
    camera.position.set(px, py, pz);
    if (camera instanceof OrthoImpl && view.mode === "top") camera.zoom = view.zoom;
    camera.updateProjectionMatrix();
    if (view.mode === "tilted") controls.maxDistance = view.distance * 4;
    controls.target.set(...view.target);
    controls.update();
  }, [boxes, camera, controls, get, mode]);

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
        <PerspectiveCamera far={4000} fov={CAMERA_FOV_DEG} makeDefault near={0.1} position={[ox, oy, oz]} />
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
        touches={mode === "top" ? TOP_TOUCH : ORBIT_TOUCH}
      />
    </>
  );
}
