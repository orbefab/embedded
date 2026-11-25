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
  let sentence: string;
  if (param.required) sentence = `${takes} (required)`;
  else if (param.default === null) sentence = `${takes}, not set by default`;
  else if (param.default !== undefined) sentence = `${takes}, ${value(param.default, param.unit)} by default`;
  else sentence = takes;
  if (param.example !== undefined) {
    const shown = typeof param.example === "number" ? si(param.example, param.unit) : param.example;
    sentence += `, example ${shown}`;
  }
  return sentence;
}

/** A byte count in B, kB or MB. */
export function size(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  if (bytes < 1_000_000) return `${(bytes / 1000).toFixed(1)} kB`;
  return `${(bytes / 1_000_000).toFixed(2)} MB`;
}

function unwrapParens(term: string): string {
  if (!term.startsWith("(") || !term.endsWith(")")) return term;
  let depth = 0;
  for (let i = 0; i < term.length; i += 1) {
    if (term[i] === "(") depth += 1;
    else if (term[i] === ")") {
      depth -= 1;
      if (depth === 0 && i < term.length - 1) return term;
    }
  }
  return depth === 0 ? term.slice(1, -1).trim() : term;
}

/** Splits an SPDX expression on top-level `AND` operators, keeping parenthesized groups intact. */
function spdxAndTerms(expression: string): string[] {
  const terms: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < expression.length; i += 1) {
    const ch = expression[i];
    if (ch === "(") depth += 1;
    else if (ch === ")") depth = Math.max(0, depth - 1);
    else if (depth === 0 && expression.startsWith(" AND ", i)) {
      terms.push(expression.slice(start, i).trim());
      i += 4;
      start = i + 1;
    }
  }
  terms.push(expression.slice(start).trim());
  return terms.filter((term) => term !== "");
}

/** Closed-box license line for a Block: the sketch's license, plus how many third-party licenses
 * the prebuilt program includes. */
export function blockLicenseSummary(expression: string): string {
  const terms = spdxAndTerms(expression);
  if (terms.length <= 1) return expression;
  const sketch = unwrapParens(terms[0] ?? expression);
  const extra = terms.length - 1;
  return `${sketch}, plus ${extra} third-party ${extra === 1 ? "license" : "licenses"}`;
}
