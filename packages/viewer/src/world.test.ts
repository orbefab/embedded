import { describe, expect, test } from "vitest";
import type { WorldQuantity } from "./protocol";
import { IDLE, nearest, nextThumb, parseValue, shownValue, stops, type Thumb, type ThumbEvent } from "./world";

const light: WorldQuantity = { name: "light", unit: "lx", min: 0, max: 100_000, start: 300, scale: { kind: "log" } };
const temperature: WorldQuantity = { name: "temperature", unit: "degC", min: -40, max: 125, start: 25, scale: { kind: "linear", step: 0.5 } };
const humidity: WorldQuantity = { name: "humidity", unit: "%RH", min: 0, max: 100, start: 50, scale: { kind: "linear", step: 1 } };

describe("stops", () => {
  test("light steps through round values from 0 to 100000 lx", () => {
    expect(stops(light)).toEqual([
      0, 1, 2, 3, 5, 10, 20, 30, 50, 100, 200, 300, 500, 1000, 2000, 3000, 5000, 10_000, 20_000, 30_000, 50_000, 100_000,
    ]);
  });

  test("temperature steps by 0.5 degC and humidity by 1 %RH, ends included", () => {
    const degrees = stops(temperature);
    expect({ count: degrees.length, first: degrees.slice(0, 3), around: degrees.slice(80, 83), last: degrees.at(-1) }).toEqual({
      count: 331,
      first: [-40, -39.5, -39],
      around: [0, 0.5, 1],
      last: 125,
    });
    expect(stops(humidity)).toEqual(Array.from({ length: 101 }, (_, k) => k));
  });

  test("the value a scene starts at is a stop when it is not round", () => {
    expect(stops({ ...light, start: 750 })).toContain(750);
    expect(stops({ ...temperature, start: 21.3 })).toContain(21.3);
  });

  test("a value between stops sits on the nearest", () => {
    const values = stops(light);
    expect([values[nearest(values, 300)], values[nearest(values, 760)], values[nearest(values, 0.2)]]).toEqual([300, 1000, 0]);
  });
});

describe("parseValue", () => {
  test("takes a number in range and refuses others as orbe set does", () => {
    expect([
      parseValue(light, " 300 "),
      parseValue(light, "0"),
      parseValue(light, "200000"),
      parseValue(light, "dark"),
      parseValue(light, ""),
      parseValue(temperature, "-41"),
    ]).toEqual([
      { ok: true, value: 300 },
      { ok: true, value: 0 },
      { ok: false, why: "light is from 0 to 100000 lx, not 200000" },
      { ok: false, why: "`dark` is not a number; write light in lx, such as 300" },
      { ok: false, why: "`` is not a number; write light in lx, such as 300" },
      { ok: false, why: "temperature is from -40 to 125 degC, not -41" },
    ]);
  });
});

const run = (events: readonly ThumbEvent[], from: Thumb = IDLE) => events.reduce(nextThumb, from);

describe("nextThumb", () => {
  test("a drag sends once, when it settles, and holds the value until a frame follows the reply", () => {
    const moved = run([
      { type: "move", value: 1000 },
      { type: "move", value: 2000 },
      { type: "move", value: 10_000 },
    ]);
    const sent = nextThumb(moved, { type: "settle", frame: 300 });
    const again = nextThumb(sent, { type: "settle", frame: 300 });
    const accepted = nextThumb(again, { type: "replied", value: 10_000, ok: true, frame: 300 });
    const framed = nextThumb(accepted, { type: "frame" });
    expect([sent, again, accepted, framed].map((thumb) => [thumb.kind, shownValue(thumb, 300)])).toEqual([
      ["sending", 10_000],
      ["sending", 10_000],
      ["landing", 10_000],
      ["idle", 300],
    ]);
  });

  test("a frame that already shows the value ends the wait at the reply", () => {
    expect(run([{ type: "send", value: 50, frame: 300 }, { type: "replied", value: 50, ok: true, frame: 50 }])).toEqual(IDLE);
  });

  test("a refused set returns the thumb to the frame", () => {
    expect(run([{ type: "send", value: 50, frame: 300 }, { type: "replied", value: 50, ok: false, frame: 300 }])).toEqual(IDLE);
  });

  test("letting go where the frame already is sends nothing", () => {
    expect(run([{ type: "move", value: 300 }, { type: "settle", frame: 300 }])).toEqual(IDLE);
    expect(run([{ type: "send", value: 300, frame: 300 }])).toEqual(IDLE);
  });

  test("a move while a set waits keeps the new position, and the old reply does not take it back", () => {
    const thumb = run([
      { type: "send", value: 50, frame: 300 },
      { type: "move", value: 1000 },
      { type: "replied", value: 50, ok: true, frame: 300 },
      { type: "frame" },
    ]);
    expect(thumb).toEqual({ kind: "moving", value: 1000 });
  });
});
