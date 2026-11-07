// The messages between a session and the viewer. A session builds `SceneMessage` once per
// `open` and `FrameMessage` after each change, and the viewer sends `Command`s back. The JSON
// keys are exactly these field names. Over a session's WebSocket the viewer sends each command
// as a `CommandMessage` and gets a `ReplyMessage`, and `ClosedMessage` is the last message.
//
// Lengths, positions and directions are in the part frame: mm, +Z up, the seating plane at
// z = 0. Other quantities are SI numbers: ohms, amperes, picoseconds.

export type Ref = `@${string}`;

export type Vec3 = readonly [number, number, number];

/** A rigid transform in the parent's frame. A point p of the child lands at
 * `at + R p`, R = Rz·Ry·Rx, with `rotate` in degrees about the parent's fixed X, Y, Z. */
export interface Placement {
  readonly at: Vec3;
  readonly rotate: Vec3;
}

/** Each param the scene or the parent part sets, after the reader parsed it: a number param
 * as a number in its SI unit (`"10k"` is `10000`), a choice param as its choice's name. File
 * params are left out. */
export type Params = Readonly<Record<string, number | string>>;

export interface InstanceBase {
  /** The role name in the parent, such as `nano` or `r_l`. */
  readonly name: string;
  /** A key of `SceneMessage.parts`. */
  readonly part: string;
  readonly params: Params;
  readonly children: readonly Child[];
  /** Why the part is not simulated: its own `not_simulated` reason in its parent's `parts`,
   * else the nearest marked ancestor's. Absent for a simulated part. */
  readonly not_simulated?: string;
}

/** A part the scene uses. `placement` is null when the scene's `layout` has no entry, and
 * the viewer lays it out for drawing only. */
export interface Instance extends InstanceBase {
  readonly placement: Placement | null;
}

/** A child of a part, placed by its parent's `layout`. A child the layout leaves out sits at
 * the parent's origin, so the reader writes the identity placement for it. */
export interface Child extends InstanceBase {
  readonly placement: Placement;
}

/** One lead of a part's body model, copied from `<id>.anchors.json`. */
export interface Anchor {
  readonly lead: number;
  /** The GLB node of the lead. A pad's node is merged away and no longer in the GLB. */
  readonly node: string;
  /** Other GLB nodes that belong to the lead, such as a header pin's housing. */
  readonly also: readonly string[];
  /** The lead's tip. */
  readonly at: Vec3;
  /** The unit direction the lead goes in when it is inserted. */
  readonly axis: Vec3;
  readonly kind: "leg" | "socket" | "pad";
}

/** `part.jsonc`'s `view`, unchanged. Every node is a GLB node name of the part's body model. */
export interface PartView {
  readonly glow?: readonly { readonly node: string; readonly by: "brightness" }[];
  /** The node takes the `tint` of the choice the param `by` picks. */
  readonly tint?: readonly { readonly node: string; readonly by: string }[];
  readonly recolour?: readonly {
    readonly nodes: readonly string[];
    readonly by: string;
    readonly rule: "iec60062-4-band";
  }[];
}

/** A joint of `part.jsonc`, with `range` read as mm. The node slides along `slide`, a unit
 * vector in the part frame, by `range` when the joint is fully driven. */
export interface Joint {
  readonly node: string;
  readonly slide: Vec3;
  readonly range: number;
}

/** One entry of `part.jsonc`'s `not_simulated`, unchanged. It names a feature the engine
 * leaves out, such as light or temperature, not a part that is dropped. The viewer does not
 * read it yet. */
export interface NotSimulated {
  readonly what: string;
  readonly why: string;
  /** The part's own pins the entry covers. */
  readonly pins?: readonly string[];
}

/** What the viewer needs of one part, keyed by part id in `SceneMessage.parts`. */
export interface PartData {
  /** The URL of `<id>.glb`, or null for a part with no body model of its own. One URL is
   * loaded once, however many instances use the part. */
  readonly model: string | null;
  readonly anchors: readonly Anchor[];
  /** Pins defined by leads, from `part.jsonc` `pins`: `"a": [1, 2]`. */
  readonly leads: Readonly<Record<string, readonly number[]>>;
  /** Labels, pins defined by child pins, from `part.jsonc` `pins`: `"D13": ["j2.1"]`. */
  readonly labels: Readonly<Record<string, readonly string[]>>;
  readonly view: PartView;
  readonly joints: Readonly<Record<string, Joint>>;
  /** A part with `press` is pressable, and a press drives the joint it names. */
  readonly controls: { readonly press?: { readonly joint: string } };
  /** `readings.brightness.full` in amperes, the current that draws a glow at intensity 1. */
  readonly readings: { readonly brightness?: { readonly full: number } };
  /** For each choice param, the `tint` of each choice that has one. */
  readonly tints: Readonly<Record<string, Readonly<Record<string, string>>>>;
  readonly not_simulated?: readonly NotSimulated[];
}

/** A pin pair. Each end is a pin ref from the scene's top, without the `@`, such as
 * `nano.D2`. A jumper inside a part is listed here too, with its ends prefixed by the path to
 * that part. Only `jumper` is drawn. */
export interface Wire {
  readonly ends: readonly [string, string];
  readonly kind: "jumper" | "trace" | "contact";
  readonly color?: string;
}

/** The refs each frame array follows, in order. */
export interface Channels {
  readonly pins: readonly Ref[];
  readonly leds: readonly Ref[];
  readonly switches: readonly Ref[];
  readonly displays?: readonly Ref[];
}

/** A pin whose net reaches no pin the engine simulates and has a reason, the one `orbe
 * check` gives for a wire to it. Every entry carries its `why`. */
export interface PinNotSimulated {
  /** The ref hover shows for the pin: the outermost label that reaches it, such as
   * `@nano.A6`, else its own pin. */
  readonly ref: Ref;
  readonly why: string;
}

export interface SceneMessage {
  readonly type: "scene";
  readonly session: string;
  readonly scene_hash: string;
  readonly circuit_hash: string;
  readonly instances: readonly Instance[];
  readonly parts: Readonly<Record<string, PartData>>;
  readonly wires: readonly Wire[];
  readonly channels: Channels;
  readonly not_simulated?: readonly PinNotSimulated[];
}

export type RunState = "paused" | "playing" | "stopped" | "faulted";

/** The circuit at `t_ps`, indexed by the scene's `channels`. */
export interface FrameMessage {
  readonly type: "frame";
  readonly seq: number;
  readonly t_ps: number;
  readonly state: RunState;
  /** 0 for low, 1 for high, per `channels.pins`. */
  readonly pins: readonly (0 | 1)[];
  /** Mean current in amperes since the previous frame, per `channels.leds`. The frame after a
   * page joins carries the present current. */
  readonly led_a: readonly number[];
  /** 1 for closed, 0 for open, per `channels.switches`. */
  readonly switches: readonly (0 | 1)[];
  /** On a frame from a `play` tick: the speed `play` runs at, always 1 for now. */
  readonly speed?: number | "max";
  /** On a frame from a `play` tick: simulated time over wall time in that tick, as the session
   * achieved it. Under 1 when the engine runs slower than real time. */
  readonly ratio?: number;
  /** Reserved until the session sends displays: per `channels.displays`, without pixels. */
  readonly displays?: readonly DisplayState[];
  /** How many warnings the session has. */
  readonly diagnostics?: number;
}

export interface DisplayState {
  /** The panel's write count, which changes when its pixels do. */
  readonly writes: number;
  readonly on: boolean;
  /** The backlight pin's last duty as a fraction from 0 to 1, or null before its first. */
  readonly brightness: number | null;
}

/** The session ended, with why, such as `` `orbe close` ended the session``. Nothing follows. */
export interface ClosedMessage {
  readonly type: "closed";
  readonly reason: string;
}

export type ToViewer = SceneMessage | FrameMessage | ClosedMessage;

/** The CLI's own input commands. The viewer sends `press`, `release`, `play` and `pause`.
 * `play` takes no speed yet. */
export type Command =
  | { readonly cmd: "press" | "release"; readonly ref: Ref }
  | { readonly cmd: "set"; readonly ref: Ref; readonly value: string }
  | { readonly cmd: "play"; readonly speed?: number | "max" }
  | { readonly cmd: "pause" };

/** The session's state in a reply. `closed` is a session with no scene open. */
export type SessionState = RunState | "closed";

export interface ReplyError {
  readonly code: string;
  readonly message: string;
  /** Empty when there is none. */
  readonly hint: string;
}

/** A command's reply: the envelope `orbe <command> --json` prints. `data` is what the command
 * returned, and is absent for a command with nothing to return. */
export type Reply = {
  readonly session: string;
  readonly t_ps: number;
  readonly state: SessionState;
  readonly data?: Readonly<Record<string, unknown>>;
} & ({ readonly ok: true } | { readonly ok: false; readonly error: ReplyError });

/** A command as the viewer sends it on a session's WebSocket. `id` is the viewer's own and
 * comes back on its reply. */
export type CommandMessage = Command & { readonly id: number };

/** The reply to the viewer's command `id`. Only the viewer that sent the command gets it. */
export interface ReplyMessage {
  readonly type: "reply";
  readonly id: number;
  readonly reply: Reply;
}

/** Everything a session's WebSocket sends. */
export type FromSession = ToViewer | ReplyMessage;

/** Where the viewer's messages come from and its commands go: a WebSocket to the session
 * process, a Worker in the sandbox, or a fixture. */
export interface SessionSource {
  /** Calls `listener` with every message from now on, and returns the unsubscribe. A source
   * sends the current scene, then the latest frame, to each new listener. */
  subscribe(listener: (message: ToViewer) => void): () => void;
  /** Rejects when the command never reached the session. */
  send(command: Command): Promise<Reply>;
}

/** The reply's error as the CLI prints it, such as `error[ended]: …` with its hint below. */
export function errorText(error: ReplyError): string {
  const head = `error[${error.code}]: ${error.message}`;
  return error.hint === "" ? head : `${head}\n  hint: ${error.hint}`;
}

/** The command as the CLI writes it, such as `press @btn1`. */
export function commandLine(command: Command): string {
  switch (command.cmd) {
    case "press":
    case "release":
      return `${command.cmd} ${command.ref}`;
    case "set":
      return `set ${command.ref} ${command.value}`;
    case "play":
      return command.speed === undefined ? "play" : `play ${command.speed}`;
    case "pause":
      return "pause";
  }
}
