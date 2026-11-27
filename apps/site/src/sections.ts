// Site-side sections for the parts catalog page. Keeping sections here avoids adding keys to
// parts/*/part.jsonc, which would bump the part format.

export const PART_SECTIONS = [
  "Boards and microcontrollers",
  "Displays",
  "Sensors",
  "Motors, relays and sound",
  "LEDs and lights",
  "Resistors, capacitors, passives and knobs",
  "Diodes, transistors, regulators and chips",
  "Breadboards and switches",
  "Connectors and sockets",
] as const;

export type PartSection = (typeof PART_SECTIONS)[number];

/** Every listed part id mapped to the section it appears under on /orbe/parts/. */
export const PART_SECTION_BY_ID: Readonly<Record<string, PartSection>> = {
  // Boards and microcontrollers
  "atmega328p-dip28": "Boards and microcontrollers",
  "esp32s3-devkitc-1": "Boards and microcontrollers",
  "nano-board": "Boards and microcontrollers",
  "uno-board": "Boards and microcontrollers",

  // Displays
  "display-7seg-ca": "Displays",
  "display-7seg-cc": "Displays",
  "hd44780-1602-i2c": "Displays",
  "lcd1602-i2c": "Displays",
  "max7219-matrix-4in1": "Displays",
  "max7219-matrix-8x8": "Displays",
  "oled-ssd1306-128x64": "Displays",
  "ssd1306-oled-128x64": "Displays",

  // Sensors
  bmp280: "Sensors",
  dht22: "Sensors",
  "gy-bmp280": "Sensors",
  "ldr-5mm": "Sensors",
  "thermistor-ntc-10k": "Sensors",

  // Motors, relays and sound
  "buzzer-12mm": "Motors, relays and sound",
  "relay-module-1ch": "Motors, relays and sound",
  "relay-srd-05vdc-sl-c": "Motors, relays and sound",
  "servo-sg90": "Motors, relays and sound",

  // LEDs and lights
  "led-3mm": "LEDs and lights",
  "led-3mm-blue": "LEDs and lights",
  "led-3mm-green": "LEDs and lights",
  "led-3mm-white": "LEDs and lights",
  "led-3mm-yellow": "LEDs and lights",
  "led-5mm": "LEDs and lights",
  "led-5mm-blue": "LEDs and lights",
  "led-5mm-green": "LEDs and lights",
  "led-5mm-rgb-ca": "LEDs and lights",
  "led-5mm-rgb-cc": "LEDs and lights",
  "led-5mm-white": "LEDs and lights",
  "led-5mm-yellow": "LEDs and lights",
  "neopixel-ring-16": "LEDs and lights",
  "neopixel-stick-8": "LEDs and lights",

  // Resistors, capacitors, passives and knobs
  "capacitor-0603": "Resistors, capacitors, passives and knobs",
  "capacitor-ceramic-disc": "Resistors, capacitors, passives and knobs",
  "capacitor-electrolytic-radial": "Resistors, capacitors, passives and knobs",
  "crystal-3215": "Resistors, capacitors, passives and knobs",
  "crystal-hc49us": "Resistors, capacitors, passives and knobs",
  "inductor-0805": "Resistors, capacitors, passives and knobs",
  "potentiometer-wh148": "Resistors, capacitors, passives and knobs",
  "ptc-fuse-1812": "Resistors, capacitors, passives and knobs",
  "resistor-0603": "Resistors, capacitors, passives and knobs",
  "resistor-array-4x0603": "Resistors, capacitors, passives and knobs",
  "resistor-axial": "Resistors, capacitors, passives and knobs",
  "resistor-sqp15": "Resistors, capacitors, passives and knobs",
  "resonator-cstce": "Resistors, capacitors, passives and knobs",
  "trimmer-3362p": "Resistors, capacitors, passives and knobs",

  // Diodes, transistors, regulators and chips
  "atmega16u2-qfn32": "Diodes, transistors, regulators and chips",
  "bss138-level-shifter": "Diodes, transistors, regulators and chips",
  "cp2102n-qfn28": "Diodes, transistors, regulators and chips",
  "diode-1n4007": "Diodes, transistors, regulators and chips",
  "hc595-dip16": "Diodes, transistors, regulators and chips",
  "lesd5d5-sod523": "Diodes, transistors, regulators and chips",
  "lm358-dip8": "Diodes, transistors, regulators and chips",
  "lm7805-to220": "Diodes, transistors, regulators and chips",
  "nmos-irf520-to220": "Diodes, transistors, regulators and chips",
  "npn-2n2222-to92": "Diodes, transistors, regulators and chips",
  "pmos-irf9540n-to220": "Diodes, transistors, regulators and chips",
  "pnp-2n3906-to92": "Diodes, transistors, regulators and chips",
  "xc6206-sot23": "Diodes, transistors, regulators and chips",
  "zener-1n4733a": "Diodes, transistors, regulators and chips",

  // Breadboards and switches
  "breadboard-400": "Breadboards and switches",
  "breadboard-830": "Breadboards and switches",
  "joystick-ky023": "Breadboards and switches",
  "tactile-switch-6mm": "Breadboards and switches",
  "tactile-switch-ntc013": "Breadboards and switches",
  "tactile-switch-smd": "Breadboards and switches",
  "tactile-switch-ts42": "Breadboards and switches",

  // Connectors and sockets
  "barrel-jack-dc21": "Connectors and sockets",
  "battery-9v": "Connectors and sockets",
  "battery-aa-4x": "Connectors and sockets",
  "dip-socket-28": "Connectors and sockets",
  "pin-header-1x15": "Connectors and sockets",
  "pin-header-1x22": "Connectors and sockets",
  "pin-header-1x3": "Connectors and sockets",
  "pin-header-1x4": "Connectors and sockets",
  "pin-header-1x5": "Connectors and sockets",
  "pin-header-1x6": "Connectors and sockets",
  "pin-header-2x3": "Connectors and sockets",
  "socket-strip-1x10": "Connectors and sockets",
  "socket-strip-1x6": "Connectors and sockets",
  "socket-strip-1x8": "Connectors and sockets",
  "usb-b-socket": "Connectors and sockets",
};

export interface PartGroup<P> {
  readonly section: PartSection;
  readonly parts: readonly P[];
}

export function sectionOfPart(id: string, table: Readonly<Record<string, PartSection>> = PART_SECTION_BY_ID): PartSection {
  const section = table[id];
  if (section === undefined) throw new Error(`part "${id}" has no section in PART_SECTION_BY_ID`);
  return section;
}

/** Groups parts in the fixed section order, preserving each section's input order and leaving out
 * empty sections. */
export function groupParts<P extends { readonly name: string; readonly section?: PartSection }>(
  parts: readonly P[],
  table: Readonly<Record<string, PartSection>> = PART_SECTION_BY_ID,
): PartGroup<P>[] {
  const buckets = new Map<PartSection, P[]>(PART_SECTIONS.map((section) => [section, []]));
  for (const part of parts) {
    const section = part.section ?? sectionOfPart(part.name, table);
    const list = buckets.get(section);
    if (list === undefined) throw new Error(`part "${part.name}" has unknown section "${section}"`);
    list.push(part);
  }
  return PART_SECTIONS.flatMap((section) => {
    const list = buckets.get(section) ?? [];
    return list.length === 0 ? [] : [{ section, parts: list }];
  });
}

/** Checks that every listed part has a section and that the table names no part that does not
 * exist in the catalog. Throws an error naming each offending part id. */
export function validatePartSections(
  listedIds: readonly string[],
  existingIds: readonly string[],
  table: Readonly<Record<string, PartSection>> = PART_SECTION_BY_ID,
): void {
  const knownSections = new Set<string>(PART_SECTIONS);
  const missing = listedIds.filter((id) => {
    const section = table[id];
    return section === undefined || !knownSections.has(section);
  });
  if (missing.length > 0) {
    throw new Error(`listed part(s) have no section in PART_SECTION_BY_ID: ${missing.join(", ")}`);
  }
  const existing = new Set(existingIds);
  const unknown = Object.keys(table).filter((id) => !existing.has(id));
  if (unknown.length > 0) {
    throw new Error(`PART_SECTION_BY_ID names part(s) that do not exist: ${unknown.join(", ")}`);
  }
}
