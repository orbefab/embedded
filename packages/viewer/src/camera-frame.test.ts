import { expect, test } from "vitest";
import { cameraPosition, frameView, union } from "./camera-frame";
import type { Box } from "./wire-geometry";

const VIEW = { width: 800, height: 600 };

function box(minX: number, minY: number, minZ: number, maxX: number, maxY: number, maxZ: number): Box {
  return { min: [minX, minY, minZ], max: [maxX, maxY, maxZ] };
}

function tilted(part: Box) {
  const view = frameView("tilted", [part], VIEW);
  if (view.mode !== "tilted") throw new Error("the tilted camera framed the top view");
  return view;
}

test("an empty scene unions to a point at the origin", () => {
  expect(union([])).toEqual({ min: [0, 0, 0], max: [0, 0, 0] });
});

test("the tilted camera sits front-left of the part and a little below overhead", () => {
  const view = tilted(box(0, 0, 0, 20, 8, 10));
  const position = cameraPosition(view);
  expect(position[0]).toBeLessThan(view.target[0]);
  expect(position[2]).toBeGreaterThan(view.target[2]);
  const rise = (position[1] - view.target[1]) / view.distance;
  expect(rise).toBeCloseTo(Math.cos((40 * Math.PI) / 180), 5);
});

test("the top view frames a scene under 16 mm across as 16 mm", () => {
  const tiny = frameView("top", [box(0, 0, 0, 1, 1, 1)], VIEW);
  const wide = frameView("top", [box(-7.5, 0, -7.5, 8.5, 1, 8.5)], VIEW);
  if (tiny.mode !== "top" || wide.mode !== "top") throw new Error("expected the top view");
  expect(tiny.zoom).toBeCloseTo(wide.zoom, 5);
  const huge = frameView("top", [box(-30, 0, -30, 30, 1, 30)], VIEW);
  if (huge.mode !== "top") throw new Error("expected the top view");
  expect(huge.zoom).toBeLessThan(tiny.zoom);
});
