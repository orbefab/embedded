import { describe, expect, test } from "vitest";
import { speedText } from "./speed";

describe("speedText", () => {
  test("formats ratios below 1 while playing", () => {
    expect(speedText("playing", 0.07)).toBe("0.07x");
    expect(speedText("playing", 0.5)).toBe("0.50x");
    expect(speedText("playing", 0.99)).toBe("0.99x");
    expect(speedText("playing", 0.004)).toBe("<0.01x");
  });

  test("hides the speed at 1, above 1, or when rounding to 1.00x", () => {
    expect(speedText("playing", 1)).toBeNull();
    expect(speedText("playing", 1.25)).toBeNull();
    expect(speedText("playing", 0.996)).toBeNull();
  });

  test("hides the speed when ratio is absent, invalid, or not playing", () => {
    expect(speedText("playing", undefined)).toBeNull();
    expect(speedText("playing", Number.NaN)).toBeNull();
    expect(speedText("playing", -0.2)).toBeNull();
    expect(speedText("paused", 0.07)).toBeNull();
    expect(speedText("stopped", 0.07)).toBeNull();
    expect(speedText("faulted", 0.07)).toBeNull();
    expect(speedText("stepping", 0.07)).toBeNull();
  });
});
