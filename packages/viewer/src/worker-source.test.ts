import { describe, expect, test, vi } from "vitest";
import type { FrameMessage, Reply, SceneMessage, ToViewer } from "./protocol";
import type { BlockFiles } from "./registry";
import { type OpenMessage, workerSource } from "./worker-source";

/** The parts of a Worker the source uses. */
class FakeWorker {
  posted: unknown[] = [];
  onmessage: ((event: { readonly data: string }) => void) | null = null;
  onerror: ((event: { readonly message: string; preventDefault(): void }) => void) | null = null;
  terminated = false;
  terminate() {
    this.terminated = true;
  }
  postMessage(data: unknown) {
    this.posted.push(typeof data === "string" ? JSON.parse(data) : data);
  }
  receive(message: unknown) {
    this.onmessage?.({ data: JSON.stringify(message) });
  }
  receiveText(text: string) {
    this.onmessage?.({ data: text });
  }
}

const glb = new Uint8Array([0x67, 0x6c, 0x54, 0x46]);
const block: BlockFiles = {
  name: "nano-button-led",
  files: new Map([
    ["scene.jsonc", new Uint8Array([123, 125])],
    ["parts/led-5mm/led-5mm.glb", glb],
  ]),
};
const scene = {
  type: "scene",
  session: "nano-button-led",
  parts: {
    "led-5mm": { model: "/parts/led-5mm/led-5mm.glb" },
    "nano-board": { model: null },
    "lost-part": { model: "/parts/lost-part/lost-part.glb" },
  },
} as unknown as SceneMessage;
const frame: FrameMessage = { type: "frame", seq: 0, t_ps: 0, state: "paused", pins: [], led_a: [], switches: [], diagnostics: 0 };
const reply: Reply = { ok: true, session: "nano-button-led", t_ps: 0, state: "playing" };
const settle = () => new Promise((done) => setTimeout(done, 0));

describe("workerSource", () => {
  test("opens the Block first, then sends the commands that waited for it", async () => {
    const worker = new FakeWorker();
    const source = workerSource(
      () => worker as unknown as Worker,
      async () => block
    );
    const played = source.send({ cmd: "play" });
    expect(worker.posted).toEqual([]);
    await settle();
    const opening = worker.posted[0] as OpenMessage;
    expect(opening.type).toBe("open");
    expect(opening.name).toBe("nano-button-led");
    expect(opening.files.map(([path]) => path)).toEqual(["scene.jsonc", "parts/led-5mm/led-5mm.glb"]);
    expect(worker.posted[1]).toEqual({ cmd: "play", id: 1 });
    worker.receive({ type: "reply", id: 1, reply });
    await expect(played).resolves.toEqual(reply);
  });

  test("serves each model the scene names from the Block's files", async () => {
    const worker = new FakeWorker();
    const source = workerSource(
      () => worker as unknown as Worker,
      async () => block
    );
    await settle();
    const got: ToViewer[] = [];
    source.subscribe((message) => got.push(message));
    worker.receive(scene);
    worker.receive(frame);
    const shown = got[0] as SceneMessage;
    const url = shown.parts["led-5mm"]?.model ?? "";
    expect(url.startsWith("blob:")).toBe(true);
    expect(new Uint8Array(await (await fetch(url)).arrayBuffer())).toEqual(glb);
    expect(shown.parts["nano-board"]?.model).toBeNull();
    expect(shown.parts["lost-part"]?.model).toBe("/parts/lost-part/lost-part.glb");
    expect(got[1]).toEqual(frame);
    worker.receive(scene);
    expect((got[2] as SceneMessage).parts["led-5mm"]?.model).toBe(url);
  });

  test("ends with why when the Block does not load, and rejects commands", async () => {
    const worker = new FakeWorker();
    const source = workerSource(
      () => worker as unknown as Worker,
      async () => {
        throw new Error("the registry has no nope");
      }
    );
    const got: ToViewer[] = [];
    source.subscribe((message) => got.push(message));
    const waiting = source.send({ cmd: "play" });
    await expect(waiting).rejects.toThrow("the Block did not load: the registry has no nope");
    expect(got).toEqual([{ type: "closed", reason: "the Block did not load: the registry has no nope" }]);
    expect(worker.posted).toEqual([]);
  });

  test("ends with the Worker's own closed, or with its failure", async () => {
    const refused = new FakeWorker();
    const one = workerSource(
      () => refused as unknown as Worker,
      async () => block
    );
    await settle();
    const got: ToViewer[] = [];
    one.subscribe((message) => got.push(message));
    refused.receive({ type: "closed", reason: "scene.jsonc:9:23: error[part_missing]: ..." });
    expect(got).toEqual([{ type: "closed", reason: "scene.jsonc:9:23: error[part_missing]: ..." }]);

    const failing = new FakeWorker();
    const two = workerSource(
      () => failing as unknown as Worker,
      async () => block
    );
    const ended: ToViewer[] = [];
    two.subscribe((message) => ended.push(message));
    failing.onerror?.({ message: "out of memory", preventDefault() {} });
    expect(ended).toEqual([{ type: "closed", reason: "the session's Worker failed: out of memory" }]);
  });

  test.each([
    ["{not json", /^the session's Worker sent text that is not JSON: /],
    ["null", /^the session's Worker sent JSON that is not a message: null$/],
    ["[1]", /^the session's Worker sent JSON that is not a message: \[1\]$/],
  ])("a message %s ends the session, terminates the Worker and revokes its model URLs", async (text, reason) => {
    const worker = new FakeWorker();
    const source = workerSource(
      () => worker as unknown as Worker,
      async () => block
    );
    await settle();
    const got: ToViewer[] = [];
    source.subscribe((message) => got.push(message));
    worker.receive(scene);
    const url = (got[0] as SceneMessage).parts["led-5mm"]?.model ?? "";
    worker.receiveText(text);
    expect(got).toHaveLength(2);
    expect((got[1] as { reason: string }).reason).toMatch(reason);
    expect(worker.terminated).toBe(true);
    await expect(fetch(url)).rejects.toThrow();
    worker.receive(scene);
    worker.receive(frame);
    expect(got).toHaveLength(2);
    await expect(source.send({ cmd: "play" })).rejects.toThrow(reason);
  });

  test("close terminates the Worker, revokes its model URLs and rejects commands", async () => {
    const worker = new FakeWorker();
    const source = workerSource(
      () => worker as unknown as Worker,
      async () => block
    );
    await settle();
    const got: ToViewer[] = [];
    source.subscribe((message) => got.push(message));
    worker.receive(scene);
    const url = (got[0] as SceneMessage).parts["led-5mm"]?.model ?? "";
    const waiting = source.send({ cmd: "play" });
    source.close();
    expect(worker.terminated).toBe(true);
    await expect(fetch(url)).rejects.toThrow();
    await expect(waiting).rejects.toThrow("the page closed the session");
    await expect(source.send({ cmd: "pause" })).rejects.toThrow("the page closed the session");
    expect(got).toHaveLength(1);
  });

  test("a scene the Worker sent before close allocates nothing once it arrives", async () => {
    const worker = new FakeWorker();
    const source = workerSource(
      () => worker as unknown as Worker,
      async () => block
    );
    await settle();
    const got: ToViewer[] = [];
    source.subscribe((message) => got.push(message));
    const created = vi.spyOn(URL, "createObjectURL");
    try {
      source.close();
      worker.receive(scene);
      expect(worker.terminated).toBe(true);
      expect(created).not.toHaveBeenCalled();
      expect(got).toEqual([]);
    } finally {
      created.mockRestore();
    }
  });

  test("a Block that loads after close is never posted", async () => {
    const worker = new FakeWorker();
    let resolve: (files: BlockFiles) => void = () => {};
    const source = workerSource(
      () => worker as unknown as Worker,
      () => new Promise((done) => (resolve = done))
    );
    source.close();
    resolve(block);
    await settle();
    expect(worker.posted).toEqual([]);
  });
});
