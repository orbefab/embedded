import { Color } from "three";
import { expect, test } from "vitest";
import { glowColour, glowIntensity, pixelGlow, shownCurrent } from "./glow";

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

function luminance(colour: Color): number {
  return 0.2126 * colour.r + 0.7152 * colour.g + 0.0722 * colour.b;
}

test("an addressable LED's brightest channel at 255 is intensity 1, in its own colour at luminance 1", () => {
  const into = new Color();
  expect(pixelGlow([255, 0, 0], into)).toBe(1);
  expect([into.r, into.g, into.b]).toEqual([1 / 0.2126, 0, 0]);
  expect(pixelGlow([0, 64, 128], into)).toBeCloseTo(128 / 255, 10);
  expect(into.g / into.b).toBeCloseTo(0.5, 10);
  expect(into.r).toBe(0);
  expect(luminance(into)).toBeCloseTo(1, 10);
});

test("an addressable LED that is black, or not latched yet, does not glow", () => {
  const into = new Color(0.5, 0.5, 0.5);
  expect(pixelGlow([0, 0, 0], into)).toBe(0);
  expect(pixelGlow([], into)).toBe(0);
  expect([into.r, into.g, into.b]).toEqual([0.5, 0.5, 0.5]);
});
