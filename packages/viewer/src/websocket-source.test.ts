import { describe, expect, test } from "vitest";
import type { FrameMessage, Reply, SceneMessage, ToViewer } from "./protocol";
import { sessionUrl, websocketSource } from "./websocket-source";

/** The parts of a WebSocket the source uses. */
class FakeSocket {
  readyState = 0;
  sent: unknown[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { readonly data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  send(data: string) {
    this.sent.push(JSON.parse(data));
  }
  close() {}
  open() {
    this.readyState = 1;
    this.onopen?.();
  }
  receive(message: unknown) {
    this.onmessage?.({ data: JSON.stringify(message) });
  }
}

const scene = { type: "scene", session: "s" } as unknown as SceneMessage;
const frame = (seq: number): FrameMessage => ({
  type: "frame",
  seq,
  t_ps: seq * 1000,
  state: "paused",
  pins: [],
  led_a: [],
  switches: [],
});
const reply: Reply = { ok: true, session: "s", t_ps: 0, state: "paused", data: { ref: "@btn1", landed_ps: 0 } };

describe("sessionUrl", () => {
  test("takes the token from the fragment", () => {
    expect(sessionUrl({ host: "127.0.0.1:4100", protocol: "http:", hash: "#t=ab12" })).toBe(
      "ws://127.0.0.1:4100/session?t=ab12"
    );
  });
  test("is null without a token", () => {
    expect(sessionUrl({ host: "localhost:5173", protocol: "http:", hash: "" })).toBeNull();
  });
});

describe("websocketSource", () => {
  test("gives a new listener the scene and the latest frame", () => {
    const socket = new FakeSocket();
    const source = websocketSource(() => socket as unknown as WebSocket);
    socket.open();
    socket.receive(scene);
    socket.receive(frame(0));
    socket.receive(frame(1));
    const got: ToViewer[] = [];
    source.subscribe((message) => got.push(message));
    expect(got).toEqual([scene, frame(1)]);
    socket.receive(frame(2));
    expect(got.at(-1)).toEqual(frame(2));
  });

  test("sends a command queued before the socket opened, and resolves it with its reply", async () => {
    const socket = new FakeSocket();
    const source = websocketSource(() => socket as unknown as WebSocket);
    const pressed = source.send({ cmd: "press", ref: "@btn1" });
    expect(socket.sent).toEqual([]);
    socket.open();
    expect(socket.sent).toEqual([{ cmd: "press", ref: "@btn1", id: 1 }]);
    socket.receive({ type: "reply", id: 1, reply });
    await expect(pressed).resolves.toEqual(reply);
  });

  test("ends with the session's reason and rejects the commands it did not answer", async () => {
    const socket = new FakeSocket();
    const source = websocketSource(() => socket as unknown as WebSocket);
    socket.open();
    const got: ToViewer[] = [];
    source.subscribe((message) => got.push(message));
    const waiting = source.send({ cmd: "play" });
    socket.receive({ type: "closed", reason: "`orbe close` ended the session" });
    socket.onclose?.();
    await expect(waiting).rejects.toThrow("`orbe close` ended the session");
    await expect(source.send({ cmd: "pause" })).rejects.toThrow("`orbe close` ended the session");
    expect(got).toEqual([{ type: "closed", reason: "`orbe close` ended the session" }]);
  });

  test("says the connection dropped when the socket closes first", () => {
    const socket = new FakeSocket();
    const source = websocketSource(() => socket as unknown as WebSocket);
    socket.onclose?.();
    const got: ToViewer[] = [];
    source.subscribe((message) => got.push(message));
    expect(got).toEqual([{ type: "closed", reason: "the connection to the session dropped" }]);
  });
});
