import { describe, expect, it } from "vitest";
import { search, type Searchable } from "./search";

const PARTS: readonly Searchable[] = [
  {
    name: "tactile-switch-6mm",
    title: "Push button, 6 mm",
    description: "Momentary 6 x 6 mm tact switch with four legs.",
    categories: ["button", "switch", "input", "momentary", "through-hole", "pushbutton", "tact", "tactile"],
  },
  {
    name: "lm358-dip8",
    title: "LM358 dual op-amp, DIP-8",
    description: "TI LM358P dual op-amp in PDIP-8.",
    categories: ["op-amp", "opamp", "amplifier", "comparator", "through-hole"],
  },
  {
    name: "hc595-dip16",
    title: "74HC595 8-bit shift register (DIP-16)",
    description: "8-bit serial-in, parallel-out shift register.",
    categories: ["74hc595", "595", "shift-register", "sipo", "latch", "serial", "output"],
  },
  {
    name: "display-7seg-ca",
    title: "0.56 in 7-segment display, common anode",
    description: "Single-digit red 7-segment display.",
    categories: ["7-segment", "common anode", "display"],
  },
  {
    name: "nano-board",
    title: "Nano (CH340 clone)",
    description: "Nano-compatible board.",
    categories: ["board", "microcontroller", "avr", "arduino", "nano"],
  },
  {
    name: "joystick-ky023",
    title: "Thumb joystick module, 2-axis with button (KY-023)",
    description: "KY-023 2-axis thumb joystick.",
    categories: ["joystick", "ky-023", "input"],
  },
];

describe("search", () => {
  it("normalizes hyphens and spaces on ids, titles and tags", () => {
    expect(search(PARTS, "pushbutton").map((p) => p.name)).toEqual(["tactile-switch-6mm"]);
    expect(search(PARTS, "push-button").map((p) => p.name)).toEqual(["tactile-switch-6mm"]);
    expect(search(PARTS, "opamp").map((p) => p.name)).toEqual(["lm358-dip8"]);
    expect(search(PARTS, "op-amp").map((p) => p.name)).toEqual(["lm358-dip8"]);
    expect(search(PARTS, "shiftregister").map((p) => p.name)).toEqual(["hc595-dip16"]);
    expect(search(PARTS, "7-seg").map((p) => p.name)).toEqual(["display-7seg-ca"]);
    expect(search(PARTS, "7seg").map((p) => p.name)).toEqual(["display-7seg-ca"]);
    expect(search(PARTS, "ky023").map((p) => p.name)).toEqual(["joystick-ky023"]);
  });

  it("does not match across word boundaries inside compacted phrases", () => {
    expect(search(PARTS, "nano").map((p) => p.name)).toEqual(["nano-board"]);
  });
});
