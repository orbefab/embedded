import { expect, test } from "vitest";
import { LAYOUT_GAP_MM, layOut } from "./layout";

test("lays unplaced parts in a row right of the placed ones, without overlap", () => {
  const placed = [{ minX: -20, minY: -9, maxX: 20, maxY: 9 }];
  const unplaced = [
    { minX: -5, minY: -1, maxX: 5, maxY: 3 },
    { minX: -3, minY: -3, maxX: 3, maxY: 3 },
  ];
  const [first, second] = layOut(placed, unplaced);
  expect(first?.at).toEqual([20 + LAYOUT_GAP_MM + 5, -1, 0]);
  expect(second?.at).toEqual([20 + LAYOUT_GAP_MM + 10 + LAYOUT_GAP_MM + 3, 0, 0]);
});

test("starts at the origin when nothing is placed", () => {
  expect(layOut([], [{ minX: -2, minY: -2, maxX: 2, maxY: 2 }])).toEqual([{ at: [2, 0, 0], rotate: [0, 0, 0] }]);
});
