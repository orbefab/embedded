import { type Command, commandLine, type FrameMessage, type SceneMessage, type SessionSource, type ToViewer } from "@orbe/viewer";

export interface FixtureSource extends SessionSource {
  /** Sends `message` to every listener and to each later one, as a session sends a new
   * scene after `open` or a frame after a change. */
  push(message: ToViewer): void;
}

/** A session stand-in. It sends `scene`, then plays `frames` one every `stepMs`, from the
 * start again after the last. With `hold`, it sends that one frame and stays on it. It logs
 * every command the viewer sends. */
export function fixtureSource(
  first: SceneMessage,
  frames: readonly FrameMessage[],
  options: { readonly stepMs: number; readonly hold?: FrameMessage }
): FixtureSource {
  let scene = first;
  const listeners = new Set<(message: ToViewer) => void>();
  let latest = options.hold ?? frames[0];
  let timer: ReturnType<typeof setInterval> | undefined;
  let index = 0;

  const broadcast = (message: ToViewer) => {
    for (const listener of listeners) listener(message);
  };

  return {
    subscribe(listener) {
      listeners.add(listener);
      listener(scene);
      if (latest !== undefined) listener(latest);
      if (timer === undefined && options.hold === undefined && frames.length > 1) {
        timer = setInterval(() => {
          index = (index + 1) % frames.length;
          latest = frames[index];
          if (latest !== undefined) broadcast(latest);
        }, options.stepMs);
      }
      return () => {
        listeners.delete(listener);
        if (listeners.size === 0) {
          clearInterval(timer);
          timer = undefined;
        }
      };
    },
    async send(command: Command) {
      console.log(commandLine(command));
      return { ok: true, session: scene.session, t_ps: latest?.t_ps ?? 0, state: latest?.state ?? "paused" };
    },
    push(message) {
      if (message.type === "scene") scene = message;
      else if (message.type === "frame") latest = message;
      broadcast(message);
    },
  };
}
