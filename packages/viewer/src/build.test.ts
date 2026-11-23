import { BoxGeometry, Group, Mesh, MeshStandardMaterial } from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";
import { afterEach, describe, expect, test, vi } from "vitest";
import { buildScene, fromGltf, tagOf } from "./build";
import type { Instance, PartData, SceneMessage } from "./protocol";

const GLB_COLOUR = "#123456";

function body(...names: string[]): Group {
  const group = new Group();
  for (const name of names) {
    const mesh = new Mesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial({ color: GLB_COLOUR }));
    mesh.name = name;
    group.add(mesh);
  }
  return group;
}

function partData(model: string, fields: Partial<PartData> = {}): PartData {
  return {
    model,
    anchors: [],
    leads: {},
    labels: {},
    view: {},
    joints: {},
    controls: {},
    tints: {},
    ...fields,
  };
}

const BANDS = ["band1", "band2", "band3", "band4"];
const parts: Record<string, PartData> = {
  resistor: partData("resistor.glb", {
    view: { recolour: [{ nodes: BANDS, by: "resistance", rule: "iec60062-4-band" }] },
  }),
  lamp: partData("lamp.glb", { view: { glow: [{ node: "lens", by: "brightness" }] } }),
  bulb: partData("lamp.glb", { view: { glow: [{ node: "body", by: "brightness" }] } }),
  pixel: partData("pixel.glb", { view: { glow: [{ node: "lens", by: "color" }] } }),
  board: partData("board.glb"),
  pot: partData("pot.glb", {
    joints: { knob: { node: "shaft", turn: [0, 0, -1], range: 300, by: { param: "position", min: 0, max: 1 } } },
  }),
};
const models = new Map([
  ["resistor.glb", body(...BANDS)],
  ["lamp.glb", body("body")],
  ["pixel.glb", body("body", "lens")],
  ["board.glb", body("pcb")],
  ["pot.glb", body("shaft")],
]);

function scene(instances: Instance[], extra: Partial<SceneMessage> = {}): SceneMessage {
  return {
    type: "scene",
    session: "test",
    scene_hash: "test",
    circuit_hash: "test",
    instances,
    parts,
    wires: [],
    channels: { pins: [], leds: [], switches: [], pixels: [] },
    ...extra,
  };
}

const at = { at: [0, 0, 0], rotate: [0, 0, 0] } as const;

function colourOf(root: Group, ref: string, node: string): string | undefined {
  const mesh = root.getObjectByName(ref)?.getObjectByName(node);
  return mesh instanceof Mesh && mesh.material instanceof MeshStandardMaterial
    ? `#${mesh.material.color.getHexString()}`
    : undefined;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("buildScene", () => {
  test("a resistor drawn with no resistance is an error on that part", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const built = buildScene(scene([{ name: "r1", part: "resistor", params: {}, placement: at, children: [] }]), models);
    expect(built.errors).toEqual(['@r1: param "resistance" is not a number']);
  });

  test("a resistance that is present and not a number is an error on that part", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const built = buildScene(
      scene([{ name: "r1", part: "resistor", params: { resistance: "220" }, placement: at, children: [] }]),
      models
    );
    expect(built.errors).toEqual(['@r1: param "resistance" is not a number']);
  });

  test("a resistance with no 4-band code keeps the model's band colours and warns once", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const built = buildScene(
      scene([
        { name: "shunt", part: "resistor", params: { resistance: 0.01 }, placement: at, children: [] },
        { name: "r1", part: "resistor", params: { resistance: 220 }, placement: at, children: [] },
      ]),
      models
    );
    expect(built.errors).toEqual([]);
    expect(BANDS.map((band) => colourOf(built.root, "@shunt", band))).toEqual(BANDS.map(() => GLB_COLOUR));
    expect(colourOf(built.root, "@r1", "band1")).toBe("#d12b2b");
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0]?.[0]).toContain("@shunt");
  });

  test("one part the viewer cannot draw is an error on that part, and the rest still draws", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const built = buildScene(
      scene(
        [
          { name: "led1", part: "lamp", params: {}, readings: { brightness: { full: 0.01 } }, placement: at, children: [] },
          { name: "nano", part: "board", params: {}, placement: at, children: [] },
        ],
        { channels: { pins: [], leds: ["@led1"], switches: [], pixels: [] } }
      ),
      models
    );
    expect(built.errors).toEqual(['@led1: lamp: the GLB has no node "lens"']);
    const led = built.root.getObjectByName("@led1");
    expect(led !== undefined && tagOf(led)?.error).toBe('lamp: the GLB has no node "lens"');
    expect(led?.getObjectByName("body")).toBeDefined();
    expect(built.root.getObjectByName("@nano")?.getObjectByName("pcb")).toBeDefined();
  });

  test("a model the viewer refused is an error on each instance that uses it", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const refused = new Map<string, Group | Error>([...models, ["board.glb", new Error("board.glb: bad root")]]);
    const built = buildScene(
      scene([
        { name: "nano", part: "board", params: {}, placement: at, children: [] },
        { name: "r1", part: "resistor", params: { resistance: 220 }, placement: at, children: [] },
      ]),
      refused
    );
    expect(built.errors).toEqual(["@nano: board.glb: bad root"]);
    expect(colourOf(built.root, "@r1", "band1")).toBe("#d12b2b");
  });

  test("a child with no placement is an error that names its ref", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const loose = { name: "r_l", part: "resistor", params: { resistance: 1000 }, placement: null, children: [] };
    const nano = { name: "nano", part: "board", params: {}, placement: at, children: [loose] } as unknown as Instance;
    const built = buildScene(scene([nano]), models);
    expect(built.errors).toEqual(["@nano.r_l: the scene message gives this child no placement"]);
    expect(built.root.getObjectByName("@nano.r_l")).toBeUndefined();
    expect(built.root.getObjectByName("@nano")?.getObjectByName("pcb")).toBeDefined();
  });

  test("two instances of one LED part glow by each instance's own full current", () => {
    const built = buildScene(
      scene(
        [
          { name: "red", part: "bulb", params: { color: "red" }, readings: { brightness: { full: 0.01 } }, placement: at, children: [] },
          { name: "blue", part: "bulb", params: { color: "blue" }, readings: { brightness: { full: 0.02 } }, placement: at, children: [] },
        ],
        { channels: { pins: [], leds: ["@red", "@blue"], switches: [], pixels: [] } }
      ),
      models
    );
    expect(built.errors).toEqual([]);
    expect(built.glows.map((glow) => glow.by)).toEqual([
      { kind: "current", full: 0.01, channel: 0 },
      { kind: "current", full: 0.02, channel: 1 },
    ]);
  });

  test("an addressable LED's lens glows by its channel on the scene's pixels", () => {
    const built = buildScene(
      scene(
        [0, 1].map((k) => ({ name: `pixel${k}`, part: "pixel", params: {}, placement: at, children: [] })),
        { channels: { pins: [], leds: [], switches: [], pixels: ["@pixel1", "@pixel0"] } }
      ),
      models
    );
    expect(built.errors).toEqual([]);
    expect(built.glows.map((glow) => glow.by)).toEqual([
      { kind: "pixel", channel: 1 },
      { kind: "pixel", channel: 0 },
    ]);
    const lens = built.root.getObjectByName("@pixel0")?.getObjectByName("lens");
    expect(lens instanceof Mesh && lens.material === built.glows[0]?.material).toBe(true);
    expect(built.glows[0]?.material.toneMapped).toBe(false);
  });

  test("an addressable LED the scene sends no colour for does not glow", () => {
    const built = buildScene(scene([{ name: "pixel0", part: "pixel", params: {}, placement: at, children: [] }]), models);
    expect(built.errors).toEqual([]);
    expect(built.glows).toEqual([]);
  });

  test("an instance carries its not_simulated reason for hover", () => {
    const built = buildScene(
      scene([
        {
          name: "nano",
          part: "board",
          params: {},
          placement: at,
          children: [{ name: "r_tx", part: "resistor", params: { resistance: 1000 }, placement: at, children: [], not_simulated: "It feeds the TX LED." }],
        },
      ]),
      models
    );
    const group = built.root.getObjectByName("@nano.r_tx");
    expect(group !== undefined && tagOf(group)?.why).toBe("It feeds the TX LED.");
  });

  test("a lead of a part that is not simulated takes a reason only from the scene's pin list", () => {
    const anchor = { lead: 1, node: "body", also: [], at: [0, 0, 0], axis: [0, 0, -1], kind: "pad" } as const;
    const withLead = { ...parts, led: partData("lamp.glb", { anchors: [anchor], leads: { a: [1] } }) };
    const led = { name: "led", part: "led", params: {}, placement: at, children: [], not_simulated: "No fit yet." };
    const reason = (extra: Partial<SceneMessage>) => {
      const lead = buildScene(scene([led], { parts: withLead, ...extra }), models).root.getObjectByName("@led")?.getObjectByName("body");
      return lead === undefined ? "no lead" : tagOf(lead);
    };
    expect(reason({})).toEqual({ ref: "@led.a" });
    expect(reason({ not_simulated: [{ ref: "@led.a", why: "Its net reaches no simulated pin." }] })).toEqual({
      ref: "@led.a",
      why: "Its net reaches no simulated pin.",
    });
  });
});

/** A GLB with one triangle, whose one root carries the glTF turn and, with `rootHasMesh`,
 * the triangle too. */
function glb(rootHasMesh: boolean): ArrayBuffer {
  const positions = new Uint8Array(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]).buffer);
  const turn = [-Math.SQRT1_2, 0, 0, Math.SQRT1_2];
  const root = rootHasMesh ? { rotation: turn, mesh: 0, children: [1] } : { rotation: turn, children: [1] };
  const json = JSON.stringify({
    asset: { version: "2.0" },
    scene: 0,
    scenes: [{ nodes: [0] }],
    nodes: [root, { name: "body", mesh: 0 }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: "VEC3", min: [0, 0, 0], max: [1, 1, 0] }],
    bufferViews: [{ buffer: 0, byteLength: positions.length }],
    buffers: [{ byteLength: positions.length }],
  });
  const text = new TextEncoder().encode(json.padEnd(Math.ceil(json.length / 4) * 4, " "));
  const length = 12 + 8 + text.length + 8 + positions.length;
  const file = new Uint8Array(length);
  const view = new DataView(file.buffer);
  view.setUint32(0, 0x46546c67, true);
  view.setUint32(4, 2, true);
  view.setUint32(8, length, true);
  view.setUint32(12, text.length, true);
  view.setUint32(16, 0x4e4f534a, true);
  file.set(text, 20);
  view.setUint32(20 + text.length, positions.length, true);
  view.setUint32(24 + text.length, 0x004e4942, true);
  file.set(positions, 28 + text.length);
  return file.buffer;
}

describe("fromGltf", () => {
  test("takes a root that carries only the turn", async () => {
    const gltf = await new GLTFLoader().parseAsync(glb(false), "");
    expect(fromGltf(gltf, "ok.glb").getObjectByName("body")).toBeDefined();
  });

  test("refuses a root that has a mesh of its own", async () => {
    const gltf = await new GLTFLoader().parseAsync(glb(true), "");
    expect(() => fromGltf(gltf, "root-mesh.glb")).toThrow("root-mesh.glb: the GLB's root node has a mesh");
  });

  test("a joint by a param turns its node half its range each way from the middle, from the live channel", () => {
    const pots = [
      { name: "pot1", part: "pot", params: { position: 0.5 }, placement: at, children: [] },
      { name: "pot2", part: "pot", params: { position: 0.75 }, placement: at, children: [] },
    ];
    const live = { ref: "@pot2.position", unit: "1", above: 0, below: 1, start: 0.75 } as const;
    const built = buildScene(scene(pots, { params: [live] }), models);
    expect(built.errors).toEqual([]);
    const [first, second] = built.turns;
    expect([first?.channel, first?.value, second?.channel, second?.value]).toEqual([-1, 0.5, 0, 0.75]);
    const degrees = (value: number) => ((second?.angle(value) ?? Number.NaN) * 180) / Math.PI;
    expect([degrees(0), degrees(0.5), degrees(1)].map((d) => Math.round(d))).toEqual([-150, 0, 150]);
    expect(second?.axis.toArray().map((v) => Math.round(v))).toEqual([0, 0, -1]);
    expect(second?.node.name).toBe("shaft");
  });
});
