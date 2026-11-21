import type { Param } from "./records";

const PREFIXES: readonly (readonly [number, string])[] = [
  [1e6, "M"],
  [1e3, "k"],
  [1, ""],
  [1e-3, "m"],
  [1e-6, "µ"],
  [1e-9, "n"],
];

/** A value in its SI unit with a prefix, as `orbe part` prints it: `0.03` A is `30 mA`. A unit
 * of `1` is a bare number. */
export function si(value: number, unit = "1"): string {
  if (unit === "1") return String(value);
  // Zero is not above any prefix, and "0 pΩ" is not how a lower bound reads.
  if (value === 0) return `0 ${unit === "ohm" ? "Ω" : unit}`;
  const [scale, prefix] = PREFIXES.find(([scale]) => Math.abs(value) >= scale) ?? [1e-12, "p"];
  const scaled = (value / scale).toFixed(6).replace(/0+$/, "").replace(/\.$/, "");
  return `${scaled} ${prefix}${unit === "ohm" ? "Ω" : unit}`;
}

const value = (v: number | string, unit?: string) => (typeof v === "number" ? si(v, unit) : `\`${v}\``);

/** A rating's name in words: `pin_current` is "Pin current". */
export function ratingName(name: string): string {
  const words = name.replaceAll("_", " ").toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** What a param takes, as the part page says it. */
export function paramSentence(param: Param): string {
  let takes: string;
  if (param.choices !== undefined) {
    const names = param.choices.map((choice) => choice.name);
    takes = names.length === 1 ? (names[0] ?? "") : `one of ${names.join(", ")}`;
  } else if (param.file !== undefined) {
    takes = param.file === "" ? "a file" : `.${param.file} file`;
  } else {
    takes = param.unit === undefined || param.unit === "1" ? "a number" : `a value in ${param.unit === "ohm" ? "Ω" : param.unit}`;
    if (param.above !== undefined) takes += ` above ${si(param.above, param.unit)}`;
    if (param.below !== undefined) takes += `${param.above === undefined ? "" : " and"} below ${si(param.below, param.unit)}`;
  }
  if (param.required) return `${takes} (required)`;
  if (param.default === null) return `${takes}, not set by default`;
  if (param.default !== undefined) return `${takes}, ${value(param.default, param.unit)} by default`;
  return takes;
}

/** A byte count in B, kB or MB. */
export function size(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  if (bytes < 1_000_000) return `${(bytes / 1000).toFixed(1)} kB`;
  return `${(bytes / 1_000_000).toFixed(2)} MB`;
}
