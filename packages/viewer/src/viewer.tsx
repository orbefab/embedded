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
import {
  type Command,
  commandLine,
  errorText,
  type FrameMessage,
  type FrameState,
  type SceneMessage,
  type SessionSource,
} from "./protocol";
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
 * around, and the commands the session refused or never got. */
function Errors({ errors }: { errors: readonly string[] }) {
  if (errors.length === 0) return null;
  return (
    <pre data-testid="errors" style={{ ...overlay, inset: "auto 0 0 0", pointerEvents: "none" }}>
      {errors.join("\n")}
    </pre>
  );
}

/** `t_ps` in seconds, to the microsecond. */
const seconds = (t_ps: number) => `${(t_ps / 1e12).toFixed(6)} s`;

/** The session's state and time, and a button that plays or pauses it. It reads frames itself,
 * so a frame redraws only this bar. */
function Controls({ source, send }: { source: SessionSource; send: (command: Command) => void }) {
  const [now, setNow] = useState<{ state: FrameState; t_ps: number } | null>(null);
  useEffect(
    () =>
      source.subscribe((message) => {
        if (message.type === "frame") setNow({ state: message.state, t_ps: message.t_ps });
      }),
    [source]
  );
  if (now === null) return null;
  const playing = now.state === "playing";
  const ended = now.state === "stopped" || now.state === "faulted";
  return (
    <div
      data-testid="controls"
      style={{
        position: "absolute",
        top: 12,
        left: 12,
        display: "flex",
        gap: 10,
        alignItems: "center",
        padding: "4px 6px 4px 10px",
        borderRadius: 6,
        background: "rgba(20, 22, 26, 0.92)",
        border: "1px solid #3a3f47",
        color: "#e6ebf2",
        font: "12px ui-monospace, monospace",
      }}
    >
      <span data-testid="state">{now.state}</span>
      <span data-testid="time">t = {seconds(now.t_ps)}</span>
      <button
        data-testid="play"
        disabled={ended}
        onClick={() => send({ cmd: playing ? "pause" : "play" })}
        style={{
          font: "inherit",
          color: "inherit",
          background: "#2a2f37",
          border: "1px solid #4a5058",
          borderRadius: 4,
          padding: "2px 10px",
          cursor: ended ? "default" : "pointer",
        }}
        type="button"
      >
        {playing ? "Pause" : "Play"}
      </button>
    </div>
  );
}

export interface ViewerProps {
  readonly source: SessionSource;
  /** Called once the canvas has its renderer, scene and camera. */
  readonly onCreated?: (state: RootState) => void;
}

/** Draws the session's scene and its frames, and sends presses, `play` and `pause` back. `T`
 * toggles the top view. */
export function Viewer({ source, onCreated }: ViewerProps) {
  const [scene, setScene] = useState<SceneMessage | null>(null);
  const [mode, setMode] = useState<CameraMode>("tilted");
  const frame = useRef<FrameMessage | null>(null);
  const hover = useMemo(createHoverStore, []);
  const [partErrors, setPartErrors] = useState<readonly string[]>([]);
  const [sendErrors, setSendErrors] = useState<readonly string[]>([]);
  const [closed, setClosed] = useState<string | null>(null);
  const onBuilt = useCallback((built: BuiltScene) => setPartErrors(built.errors), []);
  const send = useCallback(
    (command: Command) => {
      const fail = (why: string) => {
        const line = `${commandLine(command)}: ${why}`;
        console.error(line);
        setSendErrors((errors) => [...errors, line]);
      };
      source.send(command).then(
        (reply) => {
          if (!reply.ok) fail(errorText(reply.error));
        },
        (error: unknown) => fail(error instanceof Error ? error.message : String(error))
      );
    },
    [source]
  );

  useEffect(
    () =>
      source.subscribe((message) => {
        if (message.type === "scene") {
          frame.current = null;
          setSendErrors([]);
          setScene(message);
        } else if (message.type === "frame") {
          frame.current = message;
        } else {
          setClosed(message.reason);
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
    <div data-testid="viewer" style={{ position: "relative", width: "100%", height: "100%", touchAction: "none" }}>
      <ShowError>
        <Canvas dpr={[1, 1.75]} gl={{ antialias: true, alpha: false }} onCreated={onCreated}>
          <color args={["#1b1d21"]} attach="background" />
          <hemisphereLight args={["#e6ebf2", "#2a2c30", 0.9]} />
          <ambientLight intensity={0.25} />
          <directionalLight intensity={1.35} position={[40, 90, 24]} />
          {scene === null ? null : (
            <Suspense fallback={null}>
              <SceneView frame={frame} hover={hover} mode={mode} onBuilt={onBuilt} scene={scene} send={send} />
            </Suspense>
          )}
        </Canvas>
      </ShowError>
      <Errors errors={[...partErrors, ...sendErrors]} />
      {closed === null ? <Controls send={send} source={source} /> : null}
      {closed === null ? null : (
        <div
          data-testid="closed"
          style={{ ...overlay, inset: 0, display: "grid", placeItems: "center", background: "rgba(16, 17, 20, 0.8)", color: "#e6ebf2" }}
        >
          {`the session ended: ${closed}`}
        </div>
      )}
      <HoverLabel hover={hover} />
    </div>
  );
}
