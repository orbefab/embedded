// A part page draws one part with no scene of its own. A required param that has no default
// still has to be a number or a choice when the view reads it, or the viewer reports an error.
// The value comes from the part file when that file names one: the quantity its summary gives
// as the buy link's value, or the only choice, or the choice whose number a buy link contains.
import { si } from "./format";
import type { PartRecord } from "./records";

const PREFIX: Readonly<Record<string, number>> = {
  p: 1e-12,
  n: 1e-9,
  u: 1e-6,
  µ: 1e-6,
  m: 1e-3,
  k: 1e3,
  M: 1e6,
};

const UNIT: Readonly<Record<string, string>> = {
  "Ω": "ohm",
  ohm: "ohm",
  ohms: "ohm",
  F: "F",
  H: "H",
  V: "V",
  A: "A",
  s: "s",
  Hz: "Hz",
  W: "W",
};

// "The buy link is the 220 Ω value" and "the 10 kΩ value". Only the opening T varies in case.
// The prefix stays case-sensitive, so m is milli and M is mega. Ω is not a word character, so
// a word boundary would miss it. The lookahead keeps "ohm" from matching a longer word.
const BUY_VALUE = /[Tt]he buy link is the (\d+(?:\.\d+)?)(?:\s*([pnuµmkM]))?\s*(Ω|ohm|ohms|F|H|V|A|s|Hz|W)(?![A-Za-z0-9])/;

interface SceneInstance {
  params: Record<string, number | string>;
  part: string;
  children?: SceneInstance[];
}

interface SceneMessage {
  instances: SceneInstance[];
  parts: Record<string, { view?: { tint?: readonly { by?: string }[]; recolour?: readonly { by?: string }[] } }>;
}

interface Picked {
  readonly raw: number | string;
  readonly label: string;
}

function needsValue(record: PartRecord, name: string): boolean {
  const param = record.params.find((item) => item.name === name);
  return param !== undefined && param.required === true && param.default === undefined;
}

/** The quantity a summary names as what the buy link is, in the param's SI unit. */
function buyLinkValue(description: string | undefined): { value: number; unit: string } | undefined {
  const match = description?.match(BUY_VALUE);
  if (match === null || match === undefined) return undefined;
  const magnitude = Number(match[1]);
  const scale = match[2] === undefined ? 1 : PREFIX[match[2]];
  const unit = UNIT[match[3] ?? ""];
  if (scale === undefined || unit === undefined || !Number.isFinite(magnitude)) return undefined;
  return { value: magnitude * scale, unit };
}

/** A value for each required param that has no default and that the part file names. */
function pickedValues(record: PartRecord): Map<string, Picked> {
  const picked = new Map<string, Picked>();
  const quantity = buyLinkValue(record.description);
  if (quantity !== undefined) {
    const open = record.params.filter(
      (param) => param.required === true && param.default === undefined && param.unit === quantity.unit && param.choices === undefined && param.file === undefined,
    );
    if (open.length === 1) {
      const param = open[0];
      if (param !== undefined) picked.set(param.name, { raw: quantity.value, label: si(quantity.value, quantity.unit) });
    }
  }
  const urls = record.buy?.map((link) => link.url) ?? [];
  for (const param of record.params) {
    if (param.required !== true || param.default !== undefined || param.choices === undefined) continue;
    const named = param.choices.filter((choice) => choice.number !== undefined && urls.some((url) => url.includes(choice.number ?? "")));
    const choice = param.choices.length === 1 ? param.choices[0] : named.length === 1 ? named[0] : undefined;
    if (choice !== undefined) picked.set(param.name, { raw: choice.name, label: choice.name });
  }
  return picked;
}

function recolourParams(part: SceneMessage["parts"][string] | undefined): Set<string> {
  const names = new Set<string>();
  for (const entry of part?.view?.recolour ?? []) if (typeof entry.by === "string") names.add(entry.by);
  return names;
}

/** Writes each picked value onto the part page's one instance. The sentence names a resistor
 * value the view draws. A color is drawn and not named. */
export function supplyPartScene(sceneText: string, record: PartRecord): { scene: string; shown?: string } {
  const scene = JSON.parse(sceneText) as SceneMessage;
  const root = scene.instances[0];
  if (root === undefined) throw new Error(`${record.name}: the part scene has no instance`);
  root.params ??= {};
  const named = recolourParams(scene.parts[root.part]);
  const labels: string[] = [];
  for (const [name, value] of pickedValues(record)) {
    if (!needsValue(record, name) || root.params[name] !== undefined) continue;
    root.params[name] = value.raw;
    if (named.has(name)) labels.push(value.label);
  }
  return { scene: JSON.stringify(scene), shown: labels.length === 0 ? undefined : `Shown at ${labels.join(" and ")}.` };
}
