// Generated from the viewer messages in packages/session. Edit those, then run scripts/bless.sh.
import type { View } from "./view";
export type * from "./view";

/** The refs each frame array follows, in order. */
export interface Channels {
  readonly leds: readonly Ref[];
  readonly pins: readonly Ref[];
  readonly switches: readonly Ref[];
}

/** The session ended, with why, such as `` `orbe close` ended the session``. Nothing follows. */
export interface ClosedMessage {
  readonly reason: string;
  readonly type: "closed";
}

/** A command the viewer sends. The session reads it through the CLI's own table, as `orbe press
 * @btn1` would be. */
export type Command =
  | { readonly cmd: "press"; readonly ref: Ref }
  | { readonly cmd: "release"; readonly ref: Ref }
  | { readonly cmd: "play" }
  | { readonly cmd: "pause" }
  | { readonly cmd: "set"; readonly quantity: WorldName; readonly value: number };

/** A command as the viewer sends it on a session's WebSocket. `id` is the viewer's own and comes
 * back on its reply. */
export type CommandMessage = Command & {
  readonly id: number;
};

/** The circuit at `t_ps`, indexed by the scene's `channels`. */
export interface FrameMessage {
  /** How many diagnostics the session has, warnings and the fault. */
  readonly diagnostics: number;
  /** The diagnostic that faulted the session, as `orbe snapshot` prints it, such as
   * `error[solver.convergence] Newton did not converge ...`. Absent unless `state` is `faulted`. */
  readonly fault?: string;
  /** Mean current in amperes since the previous frame, per `channels.leds`. The frame after a
   * page joins carries the present current. */
  readonly led_a: readonly number[];
  /** 0 for low, 1 for high, per `channels.pins`. */
  readonly pins: readonly (0 | 1)[];
  /** On a frame from a `play` tick: simulated time over wall time in that tick, as the session
   * achieved it. Under 1 when the engine runs slower than real time. */
  readonly ratio?: number;
  readonly seq: number;
  /** On a frame from a `play` tick: the speed `play` runs at, always 1 for now. */
  readonly speed?: number;
  readonly state: FrameState;
  /** 1 for closed, 0 for open, per `channels.switches`. */
  readonly switches: readonly (0 | 1)[];
  readonly t_ps: number;
  readonly type: "frame";
  /** Each world quantity the scene declares, in table order, with its unit, range and value at
   * `t_ps`. Absent when the scene declares none. */
  readonly world?: readonly WorldReading[];
}

/** A frame's `state`: the session's, or `stepping` for a frame read while a `step` runs, which
 * no reply says. A frame needs an open scene, so it is never `closed`. */
export type FrameState = "paused" | "playing" | "stopped" | "faulted" | "stepping";

/** Everything a session's WebSocket sends. */
export type FromSession = ToViewer | ReplyMessage;

/** `@` and a dotted scene name, such as `@btn1` or `@nano.D13`. */
export type Ref = `@${string}`;

/** A command's reply: the envelope `orbe <command> --json` prints. `data` is what the command
 * returned, and is absent for a command with nothing to return. */
export type Reply = {
  readonly data?: unknown;
  readonly session: string;
  readonly state: State;
  readonly t_ps: number;
} & ({ readonly ok: true } | { readonly error: ReplyError; readonly ok: false });

/** A refused command's error, as the CLI prints it: `error[<code>]: <message>`, then the hint. */
export interface ReplyError {
  readonly code: string;
  /** Empty when there is none. */
  readonly hint: string;
  readonly message: string;
}

/** The reply to the viewer's command `id`. Only the viewer that sent the command gets it. `id`
 * is null when the command was not JSON with a number `id`. */
export interface ReplyMessage {
  readonly id: number | null;
  readonly reply: Reply;
  readonly type: "reply";
}

/** The scene a session opened, sent once per `open`, with the refs each frame array follows. */
export type SceneMessage = View & {
  readonly channels: Channels;
  readonly session: string;
  readonly type: "scene";
};

/** The session's state in a reply. `closed` is a session with no scene open. */
export type State =
  | "paused" | "faulted"
  /** No session is open. */
  | "closed"
  /** Time runs at real speed, since a `play`. */
  | "playing"
  /** The session reached its 2-hour stop. */
  | "stopped";

/** Everything a viewer gets from its session's source. */
export type ToViewer = SceneMessage | FrameMessage | ClosedMessage;

/** A world quantity's name: `temperature`, `humidity` or `light`. */
export type WorldName = "temperature" | "humidity" | "light";

/** One declared world quantity at a frame's `t_ps`. */
export interface WorldReading {
  readonly max: number;
  readonly min: number;
  readonly name: WorldName;
  /** The unit `value`, `min` and `max` are in, such as `lx`. */
  readonly unit: string;
  /** The value on the interval that ends at `t_ps`, before a change at `t_ps`. */
  readonly value: number;
}
