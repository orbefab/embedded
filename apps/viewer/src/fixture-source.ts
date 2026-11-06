import { type Command, commandLine, type FrameMessage, type SceneMessage, type SessionSource, type ToViewer } from "@orbe/viewer";

/** A session stand-in. It sends `scene`, then plays `frames` one every `stepMs`, from the
 * start again after the last. With `hold`, it sends that one frame and stays on it. It logs
 * every command the viewer sends. */
export function fixtureSource(
  scene: SceneMessage,
  frames: readonly FrameMessage[],
  options: { readonly stepMs: number; readonly hold?: FrameMessage }
): SessionSource {
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
    send(command: Command) {
      console.log(commandLine(command));
    },
  };
}
