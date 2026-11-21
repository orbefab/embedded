import { Color } from "three";
import { expect, test } from "vitest";
import { glowColour, glowIntensity, shownCurrent } from "./glow";

test("a pulse that has already fallen still draws at its peak, once", () => {
  expect(shownCurrent(0, 0.01)).toBe(0.01);
  expect(shownCurrent(0.004, 0.01)).toBe(0.01);
});

test("a current still near its peak draws as it is", () => {
  expect(shownCurrent(0.01, 0.01)).toBe(0.01);
  expect(shownCurrent(0.006, 0.01)).toBe(0.006);
  expect(shownCurrent(0, 0)).toBe(0);
});

test("full current is intensity 1, and the glow colour is luminance 1", () => {
  expect(glowIntensity(0.01, 0.01)).toBe(1);
  const colour = glowColour(new Color("#e0b0a4"));
  const luminance = 0.2126 * colour.r + 0.7152 * colour.g + 0.0722 * colour.b;
  expect(luminance).toBeCloseTo(1, 5);
});
