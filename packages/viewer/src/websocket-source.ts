import type {
  ClosedMessage,
  Command,
  FrameMessage,
  FromSession,
  Reply,
  SceneMessage,
  SessionSource,
  ToViewer,
} from "./protocol";

/** The address of the session's WebSocket for the page `orbe show` served, from the token in
 * its fragment, `#t=<token>`. `null` for a page with no token, such as the fixture page. */
export function sessionUrl(location: {
  readonly host: string;
  readonly protocol: string;
  readonly hash: string;
}): string | null {
  const token = new URLSearchParams(location.hash.slice(1)).get("t");
  if (token === null || token === "") return null;
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${location.host}/session?t=${encodeURIComponent(token)}`;
}

/** A live session, over the WebSocket `orbe show` serves. Commands sent before the socket opens
 * wait for it. Once the session closes or the socket drops, every listener gets a `closed`
 * message, and commands reject. */
export function websocketSource(open: () => WebSocket): SessionSource {
  const socket = open();
  const listeners = new Set<(message: ToViewer) => void>();
  const waiting = new Map<number, { resolve: (reply: Reply) => void; reject: (error: Error) => void }>();
  const queued: string[] = [];
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
    queued.length = 0;
    broadcast(message);
  };

  socket.onopen = () => {
    for (const text of queued) socket.send(text);
    queued.length = 0;
  };
  socket.onmessage = (event) => {
    if (typeof event.data !== "string") return;
    const message = JSON.parse(event.data) as FromSession;
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
        waiting.get(message.id)?.resolve(message.reply);
        waiting.delete(message.id);
        break;
      case "closed":
        end(message);
        break;
    }
  };
  socket.onclose = () => end({ type: "closed", reason: "the connection to the session dropped" });

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
        if (socket.readyState === WebSocket.OPEN) socket.send(text);
        else queued.push(text);
      });
    },
  };
}
