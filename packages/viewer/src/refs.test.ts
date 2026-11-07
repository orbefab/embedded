import { describe, expect, test } from "vitest";
import type { Child, PartData, SceneMessage } from "./protocol";
import { leadRef, leadsOf } from "./refs";

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
