import { describe, expect, it } from "vitest";
import { paramSentence, si } from "./format";

describe("si", () => {
  it("prints zero in the unit itself", () => {
    expect(si(0, "ohm")).toBe("0 Ω");
    expect(si(0, "V")).toBe("0 V");
    expect(si(0, "A")).toBe("0 A");
    expect(si(220, "ohm")).toBe("220 Ω");
    expect(si(0.03, "A")).toBe("30 mA");
  });
});

describe("paramSentence", () => {
  it("reads a lower bound of zero as above 0 in that unit", () => {
    expect(paramSentence({ name: "resistance", unit: "ohm", required: true, above: 0 })).toBe("a value in Ω above 0 Ω, required");
    expect(paramSentence({ name: "voltage", unit: "V", above: 0, below: 5 })).toBe("a value in V above 0 V and below 5 V");
  });
});
