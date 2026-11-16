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
import type { Command, FrameMessage, FrameState, Reply, SceneMessage } from "./protocol";
import { SceneView } from "./scene-view";
import { commandLine, errorText, type SessionSource } from "./session";
import { WORLD_STEPS, formatQuantity, snap, stepOf, valueAt, type Range } from "./world";

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
      {hovered.strip === undefined ? null : <div style={{ color: "#c9ced6" }}>{hovered.strip}</div>}
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

interface SetReply {
  readonly value: number;
  readonly landed_ps: number;
}

function acceptedSet(reply: Reply | null): SetReply | null {
  if (reply === null || !reply.ok || typeof reply.data !== "object" || reply.data === null) return null;
  const data = reply.data as { value?: unknown; landed_ps?: unknown };
  if (typeof data.value !== "number" || typeof data.landed_ps !== "number") return null;
  return { value: data.value, landed_ps: data.landed_ps };
}

/** One quantity's thumb. The number is the simulation's value. A drag previews, and the thumb
 * sends `set` once when it is released, because a `set` restarts the solve. */
function WorldSlider({
  name,
  range,
  unit,
  frameValue,
  shown,
  onCommit,
}: {
  name: string;
  range: Range;
  unit: string;
  frameValue: number;
  shown: number;
  onCommit: (value: number) => Promise<number | null>;
}) {
  const input = useRef<HTMLInputElement>(null);
  const commitRef = useRef(onCommit);
  commitRef.current = onCommit;
  const [dragStep, setDragStep] = useState<number | null>(null);
  const [held, setHeld] = useState<number | null>(null);
  useEffect(() => {
    if (held !== null && shown === held) setHeld(null);
  }, [held, shown]);
  useEffect(() => {
    const el = input.current;
    if (el === null) return;
    const preview = () => setDragStep(Number(el.value));
    const commit = () => {
      const snapped = valueAt(range, Number(el.value));
      setDragStep(null);
      void commitRef.current(snapped).then((accepted) => setHeld(accepted));
    };
    el.addEventListener("input", preview);
    el.addEventListener("change", commit);
    el.addEventListener("pointerup", commit);
    el.addEventListener("keyup", commit);
    return () => {
      el.removeEventListener("input", preview);
      el.removeEventListener("change", commit);
      el.removeEventListener("pointerup", commit);
      el.removeEventListener("keyup", commit);
    };
  }, [range.min, range.max]);
  const value = dragStep !== null ? valueAt(range, dragStep) : (held ?? shown);
  return (
    <label style={{ display: "flex", gap: 8, alignItems: "center" }}>
      <span style={{ width: 92 }}>{name}</span>
      <input
        ref={input}
        data-testid={`world-${name}`}
        type="range"
        min={0}
        max={WORLD_STEPS}
        step={1}
        value={dragStep ?? stepOf(range, value)}
        aria-label={name}
        onChange={() => {}}
        onPointerDown={(event) => event.currentTarget.setPointerCapture(event.pointerId)}
        style={{ width: 160 }}
      />
      <span data-testid={`world-${name}-value`} data-frame={String(frameValue)}>
        {formatQuantity(range, value)} {unit}
      </span>
    </label>
  );
}

/** One slider per declared world quantity. A scene that declares none draws nothing. */
function WorldPanel({
  source,
  send,
}: {
  source: SessionSource;
  send: (command: Command) => Promise<Reply | null>;
}) {
  const [rows, setRows] = useState<NonNullable<FrameMessage["world"]> | null>(null);
  const [pending, setPending] = useState<Record<string, SetReply>>({});
  const inflight = useRef<Record<string, number>>({});
  useEffect(
    () =>
      source.subscribe((message) => {
        if (message.type === "scene") {
          setRows(null);
          setPending({});
          inflight.current = {};
          return;
        }
        if (message.type !== "frame") return;
        const world = message.world ?? [];
        setRows(world);
        setPending((current) => {
          let next: Record<string, SetReply> | null = null;
          for (const [name, held] of Object.entries(current)) {
            if (message.t_ps > held.landed_ps) {
              next ??= { ...current };
              delete next[name];
            }
          }
          return next ?? current;
        });
      }),
    [source]
  );
  const commit = useCallback(
    (name: string, snapped: number): Promise<number | null> => {
      const row = rows?.find((item) => item.name === name);
      const authority = pending[name]?.value ?? row?.value;
      if (row === undefined || authority === undefined) return Promise.resolve(null);
      const resting = valueAt(row, stepOf(row, authority));
      if (snapped === snap(row, authority) || snapped === resting) return Promise.resolve(null);
      if (inflight.current[name] === snapped) return Promise.resolve(snapped);
      inflight.current[name] = snapped;
      return send({ cmd: "set", quantity: row.name, value: snapped }).then((reply) => {
        if (inflight.current[name] === snapped) delete inflight.current[name];
        const accepted = acceptedSet(reply);
        if (accepted === null) return null;
        setPending((current) => ({ ...current, [name]: accepted }));
        return accepted.value;
      });
    },
    [pending, rows, send]
  );
  if (rows === null || rows.length === 0) return null;
  return (
    <div
      data-testid="world"
      style={{
        position: "absolute",
        top: 52,
        left: 12,
        display: "flex",
        flexDirection: "column",
        gap: 6,
        padding: "8px 10px",
        borderRadius: 6,
        background: "rgba(20, 22, 26, 0.92)",
        border: "1px solid #3a3f47",
        color: "#e6ebf2",
        font: "12px ui-monospace, monospace",
      }}
    >
      {rows.map((row) => (
        <WorldSlider
          key={row.name}
          name={row.name}
          range={row}
          unit={row.unit}
          frameValue={row.value}
          shown={pending[row.name]?.value ?? row.value}
          onCommit={(value) => commit(row.name, value)}
        />
      ))}
    </div>
  );
}

/** The session's state and time, a button that plays or pauses it, and the fault below once
 * the session faulted. It reads frames itself, so a frame redraws only this bar. */
function Controls({ source, send }: { source: SessionSource; send: (command: Command) => void }) {
  const [now, setNow] = useState<{ state: FrameState; t_ps: number; fault?: string } | null>(null);
  useEffect(
    () =>
      source.subscribe((message) => {
        if (message.type === "frame") setNow({ state: message.state, t_ps: message.t_ps, fault: message.fault });
      }),
    [source]
  );
  if (now === null) return null;
  const playing = now.state === "playing";
  const ended = now.state === "stopped" || now.state === "faulted";
  return (
    <>
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
      {now.fault === undefined ? null : (
        <pre data-testid="fault" style={{ ...overlay, inset: "48px auto auto 12px", maxWidth: "calc(100% - 24px)", pointerEvents: "none" }}>
          {now.fault}
        </pre>
      )}
    </>
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
    (command: Command): Promise<Reply | null> => {
      const fail = (why: string) => {
        const line = `${commandLine(command)}: ${why}`;
        console.error(line);
        setSendErrors((errors) => [...errors, line]);
      };
      return source.send(command).then(
        (reply) => {
          if (!reply.ok) fail(errorText(reply.error));
          return reply;
        },
        (error: unknown) => {
          fail(error instanceof Error ? error.message : String(error));
          return null;
        }
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
      {closed === null ? <WorldPanel send={send} source={source} /> : null}
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
