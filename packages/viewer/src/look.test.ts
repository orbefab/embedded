import { Color } from "three";
import { describe, expect, it } from "vitest";
import { DEFAULT_CAMERA } from "./camera-frame";
import { LOOK_NAMES, LOOKS } from "./look";
import { finishOf } from "./look-materials";

describe("finishOf", () => {
  it("reads the models' tin, steel and gold as metal", () => {
    for (const hex of ["#c5c8ce", "#c9ccd1", "#b9bcc2", "#d0d3d8", "#b9bec4", "#d3d6db"]) {
      expect(finishOf(new Color(hex)), hex).toBe("metal");
    }
    expect(finishOf(new Color("#d9b44a"))).toBe("gold");
    expect(finishOf(new Color("#c8a85a"))).toBe("gold");
  });

  it("reads plastic, epoxy, ceramic and PCB as plastic", () => {
    for (const hex of ["#1a1a1a", "#f2f1ec", "#eeede6", "#c4a574", "#c9a35a", "#164e86", "#d8342a", "#e8e8e8", "#8a8d93"]) {
      expect(finishOf(new Color(hex)), hex).toBe("plastic");
    }
  });
});

describe("looks", () => {
  it("draws `current` as the viewer drew before looks", () => {
    const current = LOOKS.current;
    expect(current.surface).toBe("as-loaded");
    expect(current.outline).toBeNull();
    expect(current.camera).toBe(DEFAULT_CAMERA);
    expect(current.multisampling).toBe(0);
    expect(current.mat).toBe("texture");
  });

  it("names each look by its key", () => {
    for (const name of LOOK_NAMES) expect(LOOKS[name].name).toBe(name);
  });
});
