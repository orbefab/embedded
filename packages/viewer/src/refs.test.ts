import { describe, expect, test } from "vitest";
import type { Child, PartData, SceneMessage } from "./protocol";
import { holeAt, leadRef, leadsOf, stripText } from "./refs";

function part(fields: Partial<PartData>): PartData {
  return {
    model: null,
    anchors: [],
    leads: {},
    labels: {},
    view: {},
    joints: {},
    controls: {},
    readings: {},
    tints: {},
    ...fields,
  };
}

function child(name: string, type: string): Child {
  return { name, part: type, params: {}, placement: { at: [0, 0, 0], rotate: [0, 0, 0] }, children: [] };
}

const scene: SceneMessage = {
  type: "scene",
  session: "test",
  scene_hash: "",
  circuit_hash: "",
  instances: [
    { name: "nano", part: "board", params: {}, placement: null, children: [child("j1", "header"), child("j2", "header"), child("r_l", "resistor")] },
    { name: "btn1", part: "switch", params: {}, placement: null, children: [] },
  ],
  parts: {
    board: part({ labels: { D13: ["j2.1"], GND: ["j1.4", "j2.14"] } }),
    header: part({ leads: { "1": [1], "4": [4], "14": [14] } }),
    resistor: part({ leads: { a: [1], b: [2] } }),
    switch: part({ leads: { a: [1, 2], b: [3, 4] } }),
  },
  wires: [],
  channels: { pins: [], leds: [], switches: [] },
};

describe("leadsOf", () => {
  test("follows a label to the child pin behind it", () => {
    expect(leadsOf(scene, "nano.D13")).toEqual([{ path: ["nano", "j2"], lead: 1 }]);
  });

  test("lists every lead of a label on two pins, and of a pin on two legs", () => {
    expect(leadsOf(scene, "nano.GND")).toEqual([
      { path: ["nano", "j1"], lead: 4 },
      { path: ["nano", "j2"], lead: 14 },
    ]);
    expect(leadsOf(scene, "@btn1.a")).toEqual([
      { path: ["btn1"], lead: 1 },
      { path: ["btn1"], lead: 2 },
    ]);
  });

  test("reaches a child's own pin", () => {
    expect(leadsOf(scene, "nano.r_l.b")).toEqual([{ path: ["nano", "r_l"], lead: 2 }]);
  });

  test("reaches nothing for an unknown pin or instance", () => {
    expect(leadsOf(scene, "nano.D99")).toEqual([]);
    expect(leadsOf(scene, "led9.anode")).toEqual([]);
  });
});

describe("leadRef", () => {
  test("names a header lead by the board label on it", () => {
    expect(leadRef(scene, { path: ["nano", "j2"], lead: 1 })).toBe("@nano.D13");
    expect(leadRef(scene, { path: ["nano", "j2"], lead: 14 })).toBe("@nano.GND");
  });

  test("names a lead no label reaches by its own pin", () => {
    expect(leadRef(scene, { path: ["nano", "r_l"], lead: 1 })).toBe("@nano.r_l.a");
    expect(leadRef(scene, { path: ["btn1"], lead: 4 })).toBe("@btn1.b");
  });
});

describe("holeAt", () => {
  const board = part({
    anchors: [
      { lead: 1, node: null, also: [], at: [0, 0, 8.5], axis: [0, 0, 1], kind: "socket" },
      { lead: 2, node: null, also: [], at: [2.54, 0, 8.5], axis: [0, 0, 1], kind: "socket" },
      { lead: 3, node: null, also: [], at: [0, 2.54, 8.5], axis: [0, 0, 1], kind: "socket" },
    ],
    leads: { a1: [1], a2: [2], b1: [3] },
    strips: [["a1", "b1"]],
  });

  test("finds the hole nearest the point, and the strip it sits in", () => {
    expect(holeAt(board, [0.3, 0.2, 8.5])).toEqual({ pin: "a1", strip: ["a1", "b1"] });
    expect(holeAt(board, [0.1, 2.3, 5])).toEqual({ pin: "b1", strip: ["a1", "b1"] });
  });

  test("a hole in no strip is a strip of its own", () => {
    expect(holeAt(board, [2.4, 0.1, 8.5])).toEqual({ pin: "a2", strip: ["a2"] });
  });

  test("a point between holes, above the board or on a part with no strips is over none", () => {
    expect(holeAt(board, [1.27, 1.27, 8.5])).toBeUndefined();
    expect(holeAt(board, [0, 0, 12])).toBeUndefined();
    expect(holeAt(part({ anchors: board.anchors, leads: board.leads }), [0, 0, 8.5])).toBeUndefined();
  });
});

describe("stripText", () => {
  test("a column's strip is one run, and a rail lists its groups", () => {
    expect(stripText(["a1", "b1", "c1", "d1", "e1"], "bb.")).toBe("bb.a1 to bb.e1");
    expect(stripText(["W1", "W2", "W3", "W4", "W5", "W7", "W8", "W9", "W10", "W11"])).toBe("W1 to W5, W7 to W11");
  });
});
