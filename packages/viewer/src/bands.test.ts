import { describe, expect, test } from "vitest";
import { fourBands } from "./bands";

const black = "#161616";
const brown = "#6b3a1f";
const red = "#d12b2b";
const orange = "#ff8a1e";
const yellow = "#f2d21b";
const green = "#2f9e44";
const violet = "#7b3ff0";
const white = "#f4f4f4";
const gold = "#d4af37";
const silver = "#c0c0c0";

describe("fourBands", () => {
  test.each([
    [220, [red, red, brown, gold]],
    [10_000, [brown, black, orange, gold]],
    [1000, [brown, black, red, gold]],
    [10, [brown, black, black, gold]],
    [4.7, [yellow, violet, gold, gold]],
    [0.47, [yellow, violet, silver, gold]],
    [1_000_000, [brown, black, green, gold]],
    [99e9, [white, white, white, gold]],
  ] as const)("%d ohm", (ohms, bands) => {
    expect(fourBands(ohms)).toEqual(bands);
  });

  test("rounds to two significant digits, carrying into the multiplier", () => {
    expect(fourBands(4750)).toEqual(fourBands(4800));
    expect(fourBands(996)).toEqual([brown, black, red, gold]);
  });

  test.each([0, -1, Number.NaN, Number.POSITIVE_INFINITY, 0.05, 1e12])("refuses %d ohm", (ohms) => {
    expect(() => fourBands(ohms)).toThrow(RangeError);
  });
});
