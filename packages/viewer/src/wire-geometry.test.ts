import { expect, test } from "vitest";
import type { Vec3 } from "./protocol";
import { type Box, routeWire } from "./wire-geometry";

const down: Vec3 = [0, -1, 0];
const board: Box = { min: [-20, -8, -9], max: [20, 4, 9] };

test("a wire between two downward leads hangs below every box it passes under", () => {
  const path = routeWire(
    [
      { at: [-15, -8, 7], axis: down },
      { at: [15, -8, -7], axis: down },
    ],
    [board]
  );
  const under = path.points.filter((point) => Math.abs(point[0]) < 12);
  expect(under.length).toBeGreaterThan(0);
  for (const point of under) expect(point[1]).toBeLessThan(board.min[1]);
  expect(path.points[0]?.[1]).toBeLessThan(-8);
});

test("a wire between two upward ends arches over the boxes", () => {
  const up: Vec3 = [0, 1, 0];
  const path = routeWire(
    [
      { at: [-30, 0, 0], axis: up },
      { at: [30, 0, 0], axis: up },
    ],
    [board]
  );
  const over = path.points.filter((point) => Math.abs(point[0]) < 18);
  for (const point of over) expect(point[1]).toBeGreaterThan(board.max[1]);
});
