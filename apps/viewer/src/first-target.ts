// The first target, blocks/nano-button-led/scene.jsonc, written by hand as the scene message
// a session sends for it. Anchors come from each part's anchors file. Everything else is
// copied from the part files.
//
// The real scene has no layout, so a session would send `placement: null` for every
// instance and the viewer would lay them out in a row. These placements are the fixture's
// own, chosen so each wire is short.

import type { Anchor, Child, FrameMessage, Instance, PartData, SceneMessage } from "@orbe/viewer";
import ams1117 from "../../../parts/ams1117-sot223/ams1117-sot223.anchors.json";
import atmega328p from "../../../parts/atmega328p-tqfp32/atmega328p-tqfp32.anchors.json";
import capacitor0603 from "../../../parts/capacitor-0603/capacitor-0603.anchors.json";
import tantalum3216 from "../../../parts/capacitor-tantalum-3216/capacitor-tantalum-3216.anchors.json";
import ch340g from "../../../parts/ch340g/ch340g.anchors.json";
import crystal3215 from "../../../parts/crystal-3215/crystal-3215.anchors.json";
import diodeSod123 from "../../../parts/diode-sod123/diode-sod123.anchors.json";
import led0603 from "../../../parts/led-0603/led-0603.anchors.json";
import led5mm from "../../../parts/led-5mm/led-5mm.anchors.json";
import header1x15 from "../../../parts/pin-header-1x15/pin-header-1x15.anchors.json";
import header2x3 from "../../../parts/pin-header-2x3/pin-header-2x3.anchors.json";
import resistor0603 from "../../../parts/resistor-0603/resistor-0603.anchors.json";
import resistorAxial from "../../../parts/resistor-axial/resistor-axial.anchors.json";
import resonator from "../../../parts/resonator-cstce/resonator-cstce.anchors.json";
import tactile6mm from "../../../parts/tactile-switch-6mm/tactile-switch-6mm.anchors.json";
import tactileSmd from "../../../parts/tactile-switch-smd/tactile-switch-smd.anchors.json";
import usbMiniB from "../../../parts/usb-mini-b/usb-mini-b.anchors.json";

interface AnchorsFile {
  readonly anchors: readonly {
    lead: number;
    node: string;
    also: string[];
    at: number[];
    axis: number[];
    kind: string;
  }[];
}

function vec(values: readonly number[]): readonly [number, number, number] {
  const [x, y, z] = values;
  if (values.length !== 3 || x === undefined || y === undefined || z === undefined) {
    throw new Error(`anchors: expected three numbers, got ${JSON.stringify(values)}`);
  }
  return [x, y, z];
}

function anchors(file: AnchorsFile): Anchor[] {
  return file.anchors.map(({ kind, ...anchor }) => {
    if (kind !== "leg" && kind !== "socket" && kind !== "pad") throw new Error(`anchors: unknown kind ${kind}`);
    return { ...anchor, at: vec(anchor.at), axis: vec(anchor.axis), kind };
  });
}

/** Lead pins numbered 1 to `count`, each named by its number, as a header's are. */
function numbered(count: number): Record<string, number[]> {
  return Object.fromEntries(Array.from({ length: count }, (_, index) => [String(index + 1), [index + 1]]));
}

function part(id: string, file: AnchorsFile | null, fields: Partial<PartData>): PartData {
  return {
    model: `/parts/${id}/${id}.glb`,
    anchors: file === null ? [] : anchors(file),
    leads: {},
    labels: {},
    view: {},
    joints: {},
    controls: {},
    readings: {},
    tints: {},
    ...fields,
  };
}

const LED_FULL_A = 0.01;
const plunger = (node: string, range: number) => ({
  controls: { press: { joint: "plunger" } },
  joints: { plunger: { node, slide: [0, 0, -1] as const, range } },
});

const parts: Record<string, PartData> = {
  "nano-board": part("nano-board", null, {
    labels: {
      D1: ["j1.1"], D0: ["j1.2"], RESET: ["j1.3", "j2.13"], GND: ["j1.4", "j2.14"],
      D2: ["j1.5"], D3: ["j1.6"], D4: ["j1.7"], D5: ["j1.8"], D6: ["j1.9"], D7: ["j1.10"],
      D8: ["j1.11"], D9: ["j1.12"], D10: ["j1.13"], D11: ["j1.14"], D12: ["j1.15"],
      D13: ["j2.1"], "3V3": ["j2.2"], AREF: ["j2.3"],
      A0: ["j2.4"], A1: ["j2.5"], A2: ["j2.6"], A3: ["j2.7"], A4: ["j2.8"], A5: ["j2.9"], A6: ["j2.10"], A7: ["j2.11"],
      "5V": ["j2.12"], VIN: ["j2.15"],
    },
  }),
  "ams1117-sot223": part("ams1117-sot223", ams1117, {
    leads: { GND: [1], OUT: [2, 4], IN: [3] },
    not_simulated: [{ what: "regulator", pins: ["GND", "OUT", "IN"], why: "No AMS1117 fit yet." }],
  }),
  "atmega328p-tqfp32": part("atmega328p-tqfp32", atmega328p, {
    leads: {
      PD3: [1], PD4: [2], GND: [3, 5, 21], VCC: [4, 6], PB6: [7], PB7: [8], PD5: [9], PD6: [10], PD7: [11],
      PB0: [12], PB1: [13], PB2: [14], PB3: [15], PB4: [16], PB5: [17], AVCC: [18], ADC6: [19], AREF: [20],
      ADC7: [22], PC0: [23], PC1: [24], PC2: [25], PC3: [26], PC4: [27], PC5: [28], PC6: [29], PD0: [30],
      PD1: [31], PD2: [32],
    },
  }),
  "capacitor-0603": part("capacitor-0603", capacitor0603, { leads: { a: [1], b: [2] } }),
  "capacitor-tantalum-3216": part("capacitor-tantalum-3216", tantalum3216, { leads: { pos: [1], neg: [2] } }),
  ch340g: part("ch340g", ch340g, {
    leads: {
      GND: [1], TXD: [2], RXD: [3], V3: [4], UDP: [5], UDM: [6], XI: [7], XO: [8], CTS: [9], DSR: [10], RI: [11],
      DCD: [12], DTR: [13], RTS: [14], R232: [15], VCC: [16],
    },
    not_simulated: [
      {
        what: "USB to serial",
        pins: ["RXD", "V3", "UDP", "UDM", "XI", "XO", "CTS", "DSR", "RI", "DCD", "RTS", "R232", "VCC"],
        why: "The engine has no USB-serial chip. A serial source stands in for TXD and a steps source for DTR.",
      },
    ],
  }),
  "crystal-3215": part("crystal-3215", crystal3215, {
    leads: numbered(2),
    not_simulated: [{ what: "crystal", pins: ["1", "2"], why: "The engine's chips take their clock as a value." }],
  }),
  "diode-sod123": part("diode-sod123", diodeSod123, {
    leads: { cathode: [1], anode: [2] },
    not_simulated: [{ what: "forward drop", why: "No Schottky fit yet. A wire stands in, so the drop is 0 V." }],
  }),
  "led-0603": part("led-0603", led0603, {
    leads: { anode: [1], cathode: [2] },
    view: { glow: [{ node: "body", by: "brightness" }], tint: [{ node: "body", by: "color" }] },
    readings: { brightness: { full: LED_FULL_A } },
    tints: { color: { red: "#e0b0a4", yellow: "#e8c56a", green: "#c5d6b0" } },
    not_simulated: [{ what: "yellow and green", why: "They use the red WP7113ID fit until a fit of their own exists." }],
  }),
  "led-5mm": part("led-5mm", led5mm, {
    leads: { anode: [1], cathode: [2] },
    view: { glow: [{ node: "body", by: "brightness" }], tint: [{ node: "body", by: "color" }] },
    readings: { brightness: { full: LED_FULL_A } },
    tints: { color: { red: "#d8342a" } },
    not_simulated: [
      { what: "light", why: "The engine reports current. Brightness is the current over 10 mA, so 1 at 10 mA, and above that the LED blooms." },
      { what: "temperature", why: "The fit is at 25 C and the engine has no temperature." },
      { what: "reverse breakdown", why: "The engine's LED has no breakdown voltage." },
    ],
  }),
  "pin-header-1x15": part("pin-header-1x15", header1x15, { leads: numbered(15) }),
  "pin-header-2x3": part("pin-header-2x3", header2x3, { leads: numbered(6) }),
  "resistor-0603": part("resistor-0603", resistor0603, { leads: { a: [1], b: [2] } }),
  "resistor-axial": part("resistor-axial", resistorAxial, {
    leads: { a: [1], b: [2] },
    view: {
      recolour: [
        { nodes: ["band1-brown", "band2-black", "band3-orange", "band4-gold"], by: "resistance", rule: "iec60062-4-band" },
      ],
    },
    not_simulated: [
      { what: "tolerance", why: "The value is exact. The fourth band is drawn gold, 5 %." },
      { what: "temperature coefficient", why: "The engine has no temperature." },
    ],
  }),
  "resonator-cstce": part("resonator-cstce", resonator, {
    leads: { IN: [1], GND: [2], OUT: [3] },
    not_simulated: [
      { what: "oscillation", why: "The engine's chips take their clock as a value. The built-in loads stand in." },
    ],
  }),
  "tactile-switch-6mm": part("tactile-switch-6mm", tactile6mm, {
    leads: { a: [1, 2], b: [3, 4] },
    ...plunger("stem", 0.25),
    not_simulated: [{ what: "operating force", why: "A press is a contact change. Force and travel are drawing facts only." }],
  }),
  "tactile-switch-smd": part("tactile-switch-smd", tactileSmd, { leads: { a: [1, 2], b: [3, 4] }, ...plunger("button", 0.2) }),
  "usb-mini-b": part("usb-mini-b", usbMiniB, {
    leads: { VBUS: [1], DM: [2], DP: [3], ID: [4], GND: [5] },
    not_simulated: [
      { what: "USB data", pins: ["DM", "DP", "ID"], why: "The host's 5 V stands in for a plugged cable. USB data is not modelled." },
    ],
  }),
};

type Turn = readonly [number, number, number];
const UNDER: Turn = [180, 0, 0];

// nano-board's `parts` that are marked `not_simulated`, with their reasons.
const NANO_NOT_SIMULATED: Readonly<Record<string, string>> = {
  r_tx: "It feeds the TX LED, which follows the CH340G.",
  r_rx: "It feeds the RX LED, which follows the CH340G.",
  led_tx: "The CH340G drives it, and its model covers only TXD and DTR.",
  led_rx: "The CH340G drives it, and its model covers only TXD and DTR.",
  c_usb_a: "The clone's schematic is not cited, so its value and net are unknown.",
  c_usb_b: "The clone's schematic is not cited, so its value and net are unknown.",
  c_bulk_usb: "A rail capacitor. The engine's board example leaves rail capacitors out.",
  r_rxd: "Its far end is the CH340G's RXD, which its model does not cover.",
  c_bulk_reg: "It sits on the AMS1117 path, which is not modelled.",
  c_reg: "It sits on the AMS1117 path, which is not modelled.",
};

function child(name: string, type: string, at: Turn, rotate: Turn = [0, 0, 0], params: Child["params"] = {}): Child {
  const reason = NANO_NOT_SIMULATED[name];
  return {
    name,
    part: type,
    params,
    placement: { at, rotate },
    children: [],
    ...(reason === undefined ? {} : { not_simulated: reason }),
  };
}

// nano-board's `parts` and `layout`, with each param as the reader parses it.
const nanoChildren: Child[] = [
  child("j1", "pin-header-1x15", [0, 7.62, 0], [0, 0, 180]),
  child("j2", "pin-header-1x15", [0, -7.62, 0]),
  child("usb", "usb-mini-b", [-19.4, 0, 4.14]),
  child("icsp", "pin-header-2x3", [18.4, 2.2, 4.14]),
  child("mcu", "atmega328p-tqfp32", [-6.03, 0, 4.14], [0, 0, 45]),
  child("sw_reset", "tactile-switch-smd", [4.76, 0, 4.14]),
  child("resonator", "resonator-cstce", [0.32, -4.76, 4.14]),
  child("r_l", "resistor-0603", [8.5, 3.3, 4.14], undefined, { resistance: 1000 }),
  child("r_pwr", "resistor-0603", [8.5, 1.1, 4.14], undefined, { resistance: 1000 }),
  child("r_tx", "resistor-0603", [8.5, -1.1, 4.14], undefined, { resistance: 1000 }),
  child("r_rx", "resistor-0603", [8.5, -3.3, 4.14], undefined, { resistance: 1000 }),
  child("led_l", "led-0603", [11.3, 3.3, 4.14], undefined, { color: "yellow" }),
  child("led_pwr", "led-0603", [11.3, 1.1, 4.14], undefined, { color: "red" }),
  child("led_tx", "led-0603", [11.3, -1.1, 4.14], undefined, { color: "green" }),
  child("led_rx", "led-0603", [11.3, -3.3, 4.14], undefined, { color: "red" }),
  child("usb_serial", "ch340g", [-10.48, 0, 2.54], UNDER),
  child("regulator", "ams1117-sot223", [11.4, 2.3, 2.54], UNDER),
  child("crystal", "crystal-3215", [-14.3, 4.7, 2.54], UNDER),
  child("diode", "diode-sod123", [-14.6, -4.4, 2.54], UNDER),
  child("c_usb_a", "capacitor-0603", [-18.0, 6.55, 2.54], UNDER, { capacitance: 1e-7 }),
  child("c_usb_b", "capacitor-0603", [-18.2, -6.55, 2.54], UNDER, { capacitance: 1e-7 }),
  child("c_bulk_usb", "capacitor-tantalum-3216", [-6.2, -5.5, 2.54], [180, 0, 90], { capacitance: 1e-5 }),
  child("r_txd", "resistor-0603", [-2.86, 2.6, 2.54], UNDER, { resistance: 1000 }),
  child("r_rxd", "resistor-0603", [-2.86, 1.08, 2.54], UNDER, { resistance: 1000 }),
  child("c_reset", "capacitor-0603", [-2.86, -0.95, 2.54], UNDER, { capacitance: 1e-7 }),
  child("r_reset", "resistor-0603", [7.3, 5.15, 2.54], [180, 0, 90], { resistance: 10_000 }),
  child("c_bulk_reg", "capacitor-tantalum-3216", [10.7, -3.3, 2.54], UNDER, { capacitance: 1e-5 }),
  child("c_reg", "capacitor-0603", [13.6, -4.6, 2.54], UNDER, { capacitance: 1e-7 }),
];

const instances: Instance[] = [
  { name: "nano", part: "nano-board", params: {}, placement: { at: [0, 0, 0], rotate: [0, 0, 0] }, children: nanoChildren },
  { name: "btn1", part: "tactile-switch-6mm", params: {}, placement: { at: [34, 0, 0], rotate: [0, 0, 90] }, children: [] },
  { name: "r2", part: "resistor-axial", params: { resistance: 10_000 }, placement: { at: [9, 26, 0], rotate: [0, 0, 0] }, children: [] },
  { name: "r1", part: "resistor-axial", params: { resistance: 220 }, placement: { at: [-18, -26, 0], rotate: [0, 0, 0] }, children: [] },
  { name: "led1", part: "led-5mm", params: { color: "red" }, placement: { at: [-2, -30, 0], rotate: [0, 0, 0] }, children: [] },
];

// A session sends every wire, the Nano's 69 traces included. One trace stands in for them
// here, and like them it is not drawn.
export const firstTarget: SceneMessage = {
  type: "scene",
  session: "fixture",
  scene_hash: "fixture",
  circuit_hash: "fixture",
  instances,
  parts,
  wires: [
    { ends: ["nano.D2", "btn1.a"], kind: "jumper" },
    { ends: ["btn1.b", "nano.5V"], kind: "jumper" },
    { ends: ["nano.D2", "r2.a"], kind: "jumper" },
    { ends: ["r2.b", "nano.GND"], kind: "jumper" },
    { ends: ["nano.D13", "r1.a"], kind: "jumper" },
    { ends: ["r1.b", "led1.anode"], kind: "jumper" },
    { ends: ["led1.cathode", "nano.GND"], kind: "jumper" },
    { ends: ["nano.mcu.PB5", "nano.j2.1"], kind: "trace" },
  ],
  channels: {
    pins: ["@nano.D2", "@nano.D13"],
    leds: ["@led1", "@nano.led_l", "@nano.led_pwr"],
    switches: ["@btn1", "@nano.sw_reset"],
    pixels: [],
  },
  // The Nano's header pins whose nets reach no simulated pin, with the reasons `orbe check`
  // gives for a wire to each.
  not_simulated: [
    { ref: "@nano.3V3", why: "It is the CH340G's 3.3 V output. The engine has no USB-serial chip, so the pin supplies nothing." },
    { ref: "@nano.VIN", why: "It feeds the AMS1117, which has no model yet. The board runs from USB." },
  ],
};

// led1 and the L LED carry the currents a run of this scene measured with the button held.
// The power LED's current is illustrative.
const PWR_A = 0.003;
const frame = (seq: number, t_ps: number, pressed: boolean): FrameMessage => ({
  type: "frame",
  seq,
  t_ps,
  state: "playing",
  pins: pressed ? [1, 1] : [0, 0],
  led_a: pressed ? [0.0122465581, 0.0028787405, PWR_A] : [0, 0, PWR_A],
  switches: pressed ? [1, 0] : [0, 0],
  pixel_rgb: [],
  diagnostics: 0,
});

/** Idle, then btn1 held with D13 high, then released. */
export const script = {
  idle: frame(0, 0, false),
  pressed: frame(1, 500_000_000_000, true),
  released: frame(2, 1_500_000_000_000, false),
} as const;
