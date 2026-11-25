import { describe, expect, it } from "vitest";
import { blockLicenseSummary, paramSentence, ratingName, si } from "./format";

describe("si", () => {
  it("prints zero in the unit itself", () => {
    expect(si(0, "ohm")).toBe("0 Ω");
    expect(si(0, "V")).toBe("0 V");
    expect(si(0, "A")).toBe("0 A");
    expect(si(220, "ohm")).toBe("220 Ω");
    expect(si(0.03, "A")).toBe("30 mA");
  });
});

describe("ratingName", () => {
  it("reads a snake_case name as words", () => {
    expect(ratingName("pin_current")).toBe("Pin current");
    expect(ratingName("current")).toBe("Current");
  });
});

describe("paramSentence", () => {
  it("reads a lower bound of zero as above 0 in that unit", () => {
    expect(paramSentence({ name: "resistance", unit: "ohm", required: true, above: 0 })).toBe("a value in Ω above 0 Ω (required)");
    expect(paramSentence({ name: "resistance", unit: "ohm", required: true, above: 0, example: 220 })).toBe("a value in Ω above 0 Ω (required), example 220 Ω");
    expect(paramSentence({ name: "voltage", unit: "V", above: 0, below: 5 })).toBe("a value in V above 0 V and below 5 V");
  });

  it("names one choice, and a file, as the page shows them", () => {
    expect(paramSentence({ name: "color", required: true, choices: [{ name: "red" }] })).toBe("red (required)");
    expect(paramSentence({ name: "color", required: true, choices: [{ name: "red" }], example: "red" })).toBe("red (required), example red");
    expect(paramSentence({ name: "color", choices: [{ name: "red" }, { name: "green" }] })).toBe("one of red, green");
    expect(paramSentence({ name: "firmware", required: true, file: "hex" })).toBe(".hex file (required)");
    expect(paramSentence({ name: "image", file: "" })).toBe("a file");
  });
});

describe("blockLicenseSummary", () => {
  it("keeps a single license unchanged", () => {
    expect(blockLicenseSummary("MIT OR Apache-2.0")).toBe("MIT OR Apache-2.0");
  });

  it("summarizes a Block's sketch license and third-party license count", () => {
    expect(blockLicenseSummary("(MIT OR Apache-2.0) AND LGPL-2.1-or-later")).toBe(
      "MIT OR Apache-2.0, plus 1 third-party license",
    );
    expect(blockLicenseSummary("(MIT OR Apache-2.0) AND LGPL-2.1-or-later AND BSD-3-Clause")).toBe(
      "MIT OR Apache-2.0, plus 2 third-party licenses",
    );
    expect(
      blockLicenseSummary(
        "(MIT OR Apache-2.0) AND LGPL-2.1-or-later AND BSD-3-Clause AND GPL-3.0-or-later WITH GCC-exception-3.1",
      ),
    ).toBe("MIT OR Apache-2.0, plus 3 third-party licenses");
    expect(
      blockLicenseSummary(
        "(MIT OR Apache-2.0) AND LGPL-2.1-or-later AND Apache-2.0 AND MIT AND BSD-3-Clause AND BSD-2-Clause AND LicenseRef-newlib AND GPL-3.0-or-later WITH GCC-exception-3.1",
      ),
    ).toBe("MIT OR Apache-2.0, plus 7 third-party licenses");
  });
});
