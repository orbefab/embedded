// Adapted from maker-sandbox (MIT, Copyright 2026 Alwurts).
import { Canvas, type RootState } from "@react-three/fiber";
import {
  Component,
  type ReactNode,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import type { BuiltScene } from "./build";
import type { CameraMode } from "./camera-frame";
import { createHoverStore, type HoverStore } from "./hover";
import type { FrameMessage, SceneMessage, SessionSource } from "./protocol";
import { SceneView } from "./scene-view";

class ShowError extends Component<{ children: ReactNode }, { error: Error | null }> {
  override state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  override render() {
    const { error } = this.state;
    return error === null ? this.props.children : <pre style={overlay}>{error.message}</pre>;
  }
}

const overlay = {
  position: "absolute",
  inset: 0,
  margin: 0,
  padding: 16,
  color: "#ff8a80",
  font: "13px ui-monospace, monospace",
  whiteSpace: "pre-wrap",
} as const;

function HoverLabel({ hover }: { hover: HoverStore }) {
  const hovered = useSyncExternalStore(hover.subscribe, hover.get);
  if (hovered === null) return null;
  return (
    <div
      data-testid="hover-label"
      style={{
        position: "fixed",
        left: hovered.x + 14,
        top: hovered.y + 14,
        pointerEvents: "none",
        padding: "2px 8px",
        borderRadius: 6,
        background: "rgba(20, 22, 26, 0.92)",
        border: "1px solid #3a3f47",
        maxWidth: 360,
        color: "#e6ebf2",
        font: "12px ui-monospace, monospace",
      }}
    >
      <div>{hovered.ref}</div>
      {hovered.why === undefined ? null : <div style={{ color: "#c9ced6" }}>not simulated: {hovered.why}</div>}
      {hovered.error === undefined ? null : <div style={{ color: "#ff8a80" }}>error: {hovered.error}</div>}
    </div>
  );
}

/** The instances and wires the viewer could not draw, which the rest of the scene draws
 * around. */
function PartErrors({ errors }: { errors: readonly string[] }) {
  if (errors.length === 0) return null;
  return (
    <pre data-testid="part-errors" style={{ ...overlay, inset: "auto 0 0 0", pointerEvents: "none" }}>
      {errors.join("\n")}
    </pre>
  );
}

export interface ViewerProps {
  readonly source: SessionSource;
  /** Called once the canvas has its renderer, scene and camera. */
  readonly onCreated?: (state: RootState) => void;
}

/** Draws the session's scene and its frames, and sends presses back. `T` toggles the top
 * view. */
export function Viewer({ source, onCreated }: ViewerProps) {
  const [scene, setScene] = useState<SceneMessage | null>(null);
  const [mode, setMode] = useState<CameraMode>("tilted");
  const frame = useRef<FrameMessage | null>(null);
  const hover = useMemo(createHoverStore, []);
  const [errors, setErrors] = useState<readonly string[]>([]);
  const onBuilt = useCallback((built: BuiltScene) => setErrors(built.errors), []);

  useEffect(
    () =>
      source.subscribe((message) => {
        if (message.type === "scene") {
          frame.current = null;
          setScene(message);
        } else {
          frame.current = message;
        }
      }),
    [source]
  );
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "t" || event.key === "T") setMode((current) => (current === "top" ? "tilted" : "top"));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div style={{ position: "relative", width: "100%", height: "100%", touchAction: "none" }}>
      <ShowError>
        <Canvas dpr={[1, 1.75]} gl={{ antialias: true, alpha: false }} onCreated={onCreated}>
          <color args={["#1b1d21"]} attach="background" />
          <hemisphereLight args={["#e6ebf2", "#2a2c30", 0.9]} />
          <ambientLight intensity={0.25} />
          <directionalLight intensity={1.35} position={[40, 90, 24]} />
          {scene === null ? null : (
            <Suspense fallback={null}>
              <SceneView frame={frame} hover={hover} mode={mode} onBuilt={onBuilt} scene={scene} source={source} />
            </Suspense>
          )}
        </Canvas>
      </ShowError>
      <PartErrors errors={errors} />
      <HoverLabel hover={hover} />
    </div>
  );
}
