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
  /** UART text new since the previous frame. Absent when none arrived and the session's text is
   * still what the previous frame ended on. */
  readonly serial?: SerialText;
  /** On a frame from a `play` tick: the speed `play` runs at, always 1 for now. */
  readonly speed?: number;
  readonly state: FrameState;
  /** 1 for closed, 0 for open, per `channels.switches`. */
  readonly switches: readonly (0 | 1)[];
  readonly t_ps: number;
  readonly type: "frame";
  /** Each value per the scene's `world`: the value from `t_ps` on, or from where the quantity's
   * last `set` lands when that is later, so a `set` shows while paused. Absent when the scene
   * declares none. */
  readonly world?: readonly number[];
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

/** How a page's thumb walks a quantity's range. */
export type Scale =
  /** Even steps of `step` from `min` to `max`. */
  | { readonly kind: "linear"; readonly step: number }
  /** Round values: 0, then 1, 2, 3 and 5 times each power of ten, up to `max`. */
  | { readonly kind: "log" };

/** The scene a session opened, sent once per `open`, with the refs each frame array follows. */
export type SceneMessage = View & {
  readonly channels: Channels;
  readonly session: string;
  readonly type: "scene";
  /** Each world quantity the scene declares, in table order, which each frame's `world` follows.
   * Absent when the scene declares none. */
  readonly world?: readonly WorldQuantity[];
};

/** UART text that reached the session since the previous frame. The session keeps one serial
 * text, the bytes of every USART in the order their stop bits ended, decoded as UTF-8 the way
 * `serial contains` searches it. It does not record which USART a byte came from, so this names
 * no port. */
export interface SerialText {
  /** UTF-8 byte index in the session's serial text where `text` starts. */
  readonly from: number;
  /** Present and true when `text` replaces what a page shows. */
  readonly replace?: boolean;
  /** The new text. With `replace` absent, a page that has shown `from` bytes appends it. With
   * `replace` true, the session no longer has what the page showed past `from`, and this is the
   * tail to show instead, at most 4096 bytes. */
  readonly text: string;
}

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

/** A world quantity the scene declares, as a page shows it. */
export interface WorldQuantity {
  readonly max: number;
  readonly min: number;
  readonly name: WorldName;
  readonly scale: Scale;
  /** The value the scene gives it at t = 0. */
  readonly start: number;
  /** The unit of `min`, `max`, `start` and a frame's values, as the CLI writes it, such as `lx`. */
  readonly unit: string;
}
