import { OrbitControls } from "@react-three/drei/core/OrbitControls";
import { OrthographicCamera } from "@react-three/drei/core/OrthographicCamera";
import { PerspectiveCamera } from "@react-three/drei/core/PerspectiveCamera";
import { useThree } from "@react-three/fiber";
import { useEffect } from "react";
import { OrthographicCamera as OrthoImpl } from "three";
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
        <OrthographicCamera far={2000} makeDefault near={0.1} position={[0, 400, 0]} zoom={TOP_VIEW_ZOOM} />
      ) : (
        <PerspectiveCamera far={4000} fov={CAMERA_FOV_DEG} makeDefault near={0.1} position={[ox, oy, oz]} />
      )}
      <OrbitControls
        dampingFactor={0.12}
        enableDamping
        enableRotate={mode === "tilted"}
        keyEvents={false}
        makeDefault
        maxPolarAngle={mode === "tilted" ? Math.PI / 2.2 : 0.01}
        minDistance={MIN_DISTANCE_MM}
        minPolarAngle={mode === "tilted" ? 0.08 : 0}
      />
    </>
  );
}
