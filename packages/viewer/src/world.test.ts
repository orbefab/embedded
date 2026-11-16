import { describe, expect, test } from "vitest";
import { WORLD_STEPS, formatQuantity, logRange, stepOf, valueAt } from "./world";

const light = { min: 0, max: 100_000 };
const temperature = { min: -40, max: 125 };
const humidity = { min: 0, max: 100 };

describe("logRange", () => {
  test("a range from 0 to at least 1000 is logarithmic", () => {
    expect(logRange(light)).toBe(true);
    expect(logRange(temperature)).toBe(false);
    expect(logRange(humidity)).toBe(false);
    expect(logRange({ min: -1, max: 10_000 })).toBe(false);
  });
});

describe("valueAt", () => {
  test("the ends are the range, and a round trip keeps every step", () => {
    for (const range of [light, temperature, humidity]) {
      expect(valueAt(range, 0)).toBe(range.min);
      expect(valueAt(range, WORLD_STEPS)).toBe(range.max);
      for (let step = 0; step <= WORLD_STEPS; step += 1) {
        const value = valueAt(range, step);
        expect(valueAt(range, stepOf(range, value))).toBe(value);
      }
    }
  });

  test("0, 50, 300 and 100000 lx are different positions", () => {
    const steps = [0, 50, 300, 100_000].map((lux) => stepOf(light, lux));
    expect(new Set(steps).size).toBe(4);
    expect(valueAt(light, 0)).toBe(0);
    expect(valueAt(light, WORLD_STEPS)).toBe(100_000);
    expect(valueAt(light, stepOf(light, 50))).toBe(50);
    expect(Math.abs(valueAt(light, stepOf(light, 300)) - 300)).toBeLessThanOrEqual(1);
  });
});

describe("formatQuantity", () => {
  test("drops trailing zeros", () => {
    expect(formatQuantity(light, 300)).toBe("300");
    expect(formatQuantity(temperature, 25)).toBe("25");
    expect(formatQuantity(temperature, 25.5)).toBe("25.5");
    expect(formatQuantity({ min: 0, max: 1 }, 0.5)).toBe("0.5");
  });
});
