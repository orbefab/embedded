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
  closed = false;
  close() {
    this.closed = true;
  }
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
  pixel_rgb: [],
  diagnostics: 0,
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

  test("a late listener sees the serial text the session already delivered", () => {
    const socket = new FakeSocket();
    const source = websocketSource(() => socket as unknown as WebSocket);
    socket.open();
    const live: ToViewer[] = [];
    source.subscribe((message) => live.push(message));
    socket.receive(scene);
    socket.receive({ ...frame(0), serial: { from: 0, text: "light 1\r\n" } });
    socket.receive({ ...frame(1), serial: { from: "light 1\r\n".length, text: "dark 2\r\n" } });
    socket.receive(frame(2));
    expect(live.flatMap((message) => (message.type === "frame" ? [message.serial?.text] : []))).toEqual([
      "light 1\r\n",
      "dark 2\r\n",
      undefined,
    ]);
    const late: ToViewer[] = [];
    source.subscribe((message) => late.push(message));
    const replay = late[1] as FrameMessage;
    expect(replay.seq).toBe(2);
    expect(replay.serial).toEqual({ from: 0, text: "light 1\r\ndark 2\r\n", replace: true });
  });

  test("a late listener sees each screen's last glass, which frames carry only when it changes", () => {
    const socket = new FakeSocket();
    const source = websocketSource(() => socket as unknown as WebSocket);
    socket.open();
    socket.receive(scene);
    const a = { width: 1, height: 1, rgba: "AAAA/w==", lit: false };
    const b = { width: 1, height: 1, rgba: "////fw==", lit: false };
    socket.receive({ ...frame(0), screens: [a, a] });
    socket.receive({ ...frame(1), screens: [null, b] });
    socket.receive(frame(2));
    const late: ToViewer[] = [];
    source.subscribe((message) => late.push(message));
    const replay = late[1] as FrameMessage;
    expect(replay.seq).toBe(2);
    expect(replay.screens).toEqual([a, b]);
    socket.receive(scene);
    socket.receive(frame(3));
    const fresh: ToViewer[] = [];
    source.subscribe((message) => fresh.push(message));
    expect((fresh[1] as FrameMessage).screens).toBeUndefined();
  });

  test("a late listener sees the tail when earlier bytes were dropped", () => {
    const socket = new FakeSocket();
    const source = websocketSource(() => socket as unknown as WebSocket);
    socket.open();
    socket.receive(scene);
    const head = "A".repeat(100);
    const body = "B".repeat(5000);
    socket.receive({ ...frame(0), serial: { from: 0, text: head } });
    socket.receive({ ...frame(1), serial: { from: head.length, text: body } });
    const late: ToViewer[] = [];
    source.subscribe((message) => late.push(message));
    const replay = late[1] as FrameMessage;
    const text = replay.serial?.text ?? "";
    expect(replay.serial?.replace).toBe(true);
    expect(replay.serial?.trimmed).toBe(true);
    expect(text.length).toBeLessThanOrEqual(4096);
    expect((head + body).endsWith(text)).toBe(true);
    expect(text.startsWith("B")).toBe(true);
    expect((replay.serial?.from ?? -1) + new TextEncoder().encode(text).length).toBe(head.length + body.length);
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

  test("ends with why on text that is not JSON, and close closes the socket", () => {
    const socket = new FakeSocket();
    const source = websocketSource(() => socket as unknown as WebSocket);
    socket.open();
    const got: ToViewer[] = [];
    source.subscribe((message) => got.push(message));
    socket.onmessage?.({ data: "{not json" });
    expect((got[0] as { reason: string }).reason).toMatch(/^the session sent text that is not JSON: /);
    source.close();
    expect(socket.closed).toBe(true);
  });
});
