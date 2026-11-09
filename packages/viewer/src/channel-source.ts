import type { ClosedMessage, Command, FrameMessage, FromSession, Reply, SceneMessage, ToViewer } from "./protocol";
import type { SessionSource } from "./session";

/** What a channel to a session tells its source. */
export interface Inbox {
  /** One message from the session. */
  message(message: FromSession): void;
  /** The channel ended without the session's own `closed`, with why. */
  drop(reason: string): void;
}

/** An open channel: how to send one command's JSON text on it, and how to free it. */
export interface Channel {
  send(text: string): void;
  close(): void;
}

/** Parses one message's JSON text from a session, or drops the channel with why. A message is
 * a JSON object with a string `type`. */
export function receive(inbox: Inbox, text: string, from: string): void {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    inbox.drop(`${from} sent text that is not JSON: ${error instanceof Error ? error.message : String(error)}`);
    return;
  }
  if (typeof parsed !== "object" || parsed === null || typeof (parsed as { type?: unknown }).type !== "string") {
    inbox.drop(`${from} sent JSON that is not a message: ${text.slice(0, 80)}`);
    return;
  }
  inbox.message(parsed as FromSession);
}

/** A session over a channel that carries its messages: `connect` opens the channel. A new
 * listener gets the current scene, then the latest frame. Once the session closes or the
 * channel drops, every listener gets a `closed` message, commands reject, and listeners get
 * nothing more. A drop also frees the channel, as `close` does. */
export function channelSource(connect: (inbox: Inbox) => Channel): SessionSource {
  const listeners = new Set<(message: ToViewer) => void>();
  const waiting = new Map<number, { resolve: (reply: Reply) => void; reject: (error: Error) => void }>();
  let scene: SceneMessage | null = null;
  let frame: FrameMessage | null = null;
  let closed: ClosedMessage | null = null;
  let next = 1;

  const broadcast = (message: ToViewer) => {
    for (const listener of listeners) listener(message);
  };
  const end = (message: ClosedMessage) => {
    if (closed !== null) return;
    closed = message;
    for (const { reject } of waiting.values()) reject(new Error(message.reason));
    waiting.clear();
    broadcast(message);
  };

  const channel: Channel = connect({
    message(message) {
      if (closed !== null) return;
      switch (message.type) {
        case "scene":
          scene = message;
          frame = null;
          broadcast(message);
          break;
        case "frame":
          frame = message;
          broadcast(message);
          break;
        case "reply":
          if (message.id === null) break;
          waiting.get(message.id)?.resolve(message.reply);
          waiting.delete(message.id);
          break;
        case "closed":
          end(message);
          break;
      }
    },
    drop(reason) {
      end({ type: "closed", reason });
      channel.close();
    },
  });

  return {
    subscribe(listener) {
      listeners.add(listener);
      if (scene !== null) listener(scene);
      if (frame !== null) listener(frame);
      if (closed !== null) listener(closed);
      return () => {
        listeners.delete(listener);
      };
    },
    send(command: Command) {
      if (closed !== null) return Promise.reject(new Error(closed.reason));
      const id = next++;
      const text = JSON.stringify({ ...command, id });
      return new Promise<Reply>((resolve, reject) => {
        waiting.set(id, { resolve, reject });
        channel.send(text);
      });
    },
    close() {
      listeners.clear();
      end({ type: "closed", reason: "the page closed the session" });
      channel.close();
    },
  };
}
