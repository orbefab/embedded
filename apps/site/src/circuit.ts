// A Block's parts and a part's "used in" list, both from the registry and the scene files.
// The card count and "Parts in this circuit" are both `placed.length`. A reference has no
// role here: the registry does not record one.

import { si } from "./format";
import { parseJsonc } from "./jsonc";
import { at } from "./paths";
import { dependencyId, type BlockRecord, type Item, type Param, type PartRecord, type Real } from "./records";

/** One reference a Block's scene places. */
export interface PlacedPart {
  readonly ref: string;
  readonly id: string;
  /** The scene's params, in the scene's order, already in words (`220 Ω`, `red`, `fade.hex`). */
  readonly values: readonly string[];
}

/** A catalog part folded under a board. */
export interface FoldedPart {
  readonly id: string;
  readonly title: string;
}

/** The board a Block runs on, and the parts that board brings with it. */
export interface BoardFold {
  readonly id: string;
  readonly title: string;
  readonly own: readonly FoldedPart[];
}

/** A text file the Code view shows. Sketch, scene and test, in that order. */
export interface SourceFile {
  readonly path: string;
  readonly text: string;
  readonly bytes: number;
  readonly license: string;
  readonly sha256: string;
  readonly group: "Sketch" | "Scene" | "Test";
}

/** Everything the Block page knows about the circuit, computed once from the record and its scene. */
export interface BlockCircuit {
  readonly placed: readonly PlacedPart[];
  readonly board: BoardFold;
  /** Lines `parts.lock` gets: the Block record's registry dependencies. */
  readonly lockCount: number;
  /** `expect` lines in the Block's tests. */
  readonly checks: number;
  readonly sources: readonly SourceFile[];
}

/** One place a part is placed in a Block. */
export interface UsedRef {
  readonly ref: string;
  readonly values: readonly string[];
}

/** A Block whose scene places this part. */
export interface UsedInBlock {
  readonly id: string;
  readonly title: string;
  readonly href: string;
  readonly refs: readonly UsedRef[];
}

/** A board that carries this part inside it. */
export interface OnBoard {
  readonly id: string;
  readonly title: string;
  readonly href: string;
}

/** Where a part shows up. `blocks` comes from scenes. `boards` comes from board dependencies. */
export interface PartUse {
  readonly blocks: readonly UsedInBlock[];
  readonly boards: readonly OnBoard[];
}

const PREFIXES: readonly (readonly [string, number])[] = [
  ["p", 1e-12],
  ["n", 1e-9],
  ["u", 1e-6],
  ["µ", 1e-6],
  ["μ", 1e-6],
  ["m", 1e-3],
  ["k", 1e3],
  ["M", 1e6],
  ["G", 1e9],
];

function writtenUnits(unit: string): readonly string[] {
  if (unit === "ohm") return ["ohm", "ohms", "Ω"];
  if (unit === "1") return [];
  return [unit];
}

/** A scene amount such as `220`, `2.2k` or `10k`, in `unit`. Undefined when it is not one. */
function parseAmount(text: string, unit: string): number | undefined {
  const match = /^(-?(?:\d+\.?\d*|\.\d+))(?:[eE]([+-]?\d+))?\s*(.*)$/.exec(text.trim());
  if (match === null) return undefined;
  const raw = match[1];
  if (raw === undefined) return undefined;
  const mantissa = Number(raw);
  const exp = match[2] === undefined ? 0 : Number(match[2]);
  if (!Number.isFinite(mantissa) || !Number.isFinite(exp)) return undefined;
  const rest = (match[3] ?? "").trim();
  const units = writtenUnits(unit);
  if (rest === "" || units.includes(rest)) return mantissa * 10 ** exp;
  const prefix = PREFIXES.find(([name]) => rest === name || (rest.startsWith(name) && units.includes(rest.slice(name.length))));
  if (prefix === undefined) return undefined;
  return mantissa * 10 ** exp * prefix[1];
}

function formatValue(raw: string | number, param: Param | undefined): string {
  const text = String(raw);
  if (text.includes("/")) return text.slice(text.lastIndexOf("/") + 1);
  if (param?.choices?.some((choice) => choice.name === text)) return text;
  const unit = param?.unit;
  if (unit !== undefined && unit !== "") {
    const amount = parseAmount(text, unit);
    if (amount !== undefined) return si(amount, unit);
  }
  return text;
}

function valueTexts(id: string, fields: Readonly<Record<string, unknown>>, parts: readonly Item<PartRecord>[]): readonly string[] {
  const params = parts.find((item) => item.record.name === id)?.record.params ?? [];
  const shown = Object.entries(fields).flatMap(([key, raw]) => {
    if (key === "type" || (typeof raw !== "string" && typeof raw !== "number")) return [];
    const param = params.find((entry) => entry.name === key);
    return [{ name: key, text: formatValue(raw, param) }];
  });
  if (shown.length <= 1) return shown.map((entry) => entry.text);
  return shown.map((entry, index) => (index === 0 ? entry.text : `${entry.name} ${entry.text}`));
}

function placedParts(record: BlockRecord, texts: Readonly<Record<string, string>>, parts: readonly Item<PartRecord>[]): readonly PlacedPart[] {
  const file = record.files.find((entry) => entry.path === "scene.jsonc");
  const text = file === undefined ? undefined : texts[file.sha256];
  if (text === undefined) throw new Error(`${record.name} has no scene.jsonc`);
  const scene = parseJsonc(text);
  const raw = scene !== null && typeof scene === "object" && "parts" in scene ? scene.parts : undefined;
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) throw new Error(`${record.name} scene has no parts`);
  return Object.entries(raw).map(([ref, body]) => {
    if (body === null || typeof body !== "object" || Array.isArray(body)) throw new Error(`${record.name} scene part ${ref} is not an object`);
    const fields = body as Record<string, unknown>;
    const id = fields.type;
    if (typeof id !== "string") throw new Error(`${record.name} scene part ${ref} has no type`);
    return { ref, id, values: valueTexts(id, fields, parts) };
  });
}

function boardFold(id: string, parts: readonly Item<PartRecord>[]): BoardFold {
  const record = parts.find((item) => item.record.name === id)?.record;
  if (record === undefined) throw new Error(`board ${id} is not a part`);
  const seen = new Set<string>();
  const own: FoldedPart[] = [];
  for (const dependency of record.registryDependencies) {
    const partId = dependencyId(dependency);
    if (seen.has(partId)) continue;
    seen.add(partId);
    own.push({ id: partId, title: parts.find((item) => item.record.name === partId)?.record.title ?? partId });
  }
  return { id, title: record.title, own };
}

function sourceGroup(path: string): SourceFile["group"] | undefined {
  if (path === "scene.jsonc") return "Scene";
  if (path.startsWith("tests/")) return "Test";
  if (/^firmware\/.+\.ino$/.test(path)) return "Sketch";
  return undefined;
}

function groupOrder(group: SourceFile["group"]): number {
  if (group === "Sketch") return 0;
  if (group === "Scene") return 1;
  return 2;
}

function sourcesOf(record: BlockRecord, texts: Readonly<Record<string, string>>): readonly SourceFile[] {
  const sources: SourceFile[] = [];
  for (const file of record.files) {
    const group = sourceGroup(file.path);
    const text = texts[file.sha256];
    if (group === undefined || text === undefined) continue;
    sources.push({ path: file.path, text, bytes: file.bytes, license: file.license, sha256: file.sha256, group });
  }
  return sources.sort((a, b) => groupOrder(a.group) - groupOrder(b.group) || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
}

function checkCount(record: BlockRecord, texts: Readonly<Record<string, string>>): number {
  let count = 0;
  for (const file of record.files) {
    if (!file.path.startsWith("tests/")) continue;
    const text = texts[file.sha256];
    if (text === undefined) continue;
    for (const line of text.split("\n")) if (/^\s*expect\b/.test(line)) count += 1;
  }
  return count;
}

function circuitOf(record: BlockRecord, texts: Readonly<Record<string, string>>, parts: readonly Item<PartRecord>[]): BlockCircuit {
  return {
    placed: placedParts(record, texts, parts),
    board: boardFold(record.board, parts),
    lockCount: record.registryDependencies.length,
    checks: checkCount(record, texts),
    sources: sourcesOf(record, texts),
  };
}

const isBoard = (record: PartRecord) => record.categories?.includes("board") === true;

/** Every Block's circuit, and for each part the Blocks and boards that carry it. */
export function readCatalogUse(
  blocks: readonly Item<BlockRecord>[],
  parts: readonly Item<PartRecord>[],
  texts: Readonly<Record<string, string>>,
): { readonly circuits: Readonly<Record<string, BlockCircuit>>; readonly uses: Readonly<Record<string, PartUse>> } {
  const circuits: Record<string, BlockCircuit> = {};
  const byPart = new Map<string, UsedInBlock[]>();
  for (const { record } of blocks) {
    const circuit = circuitOf(record, texts, parts);
    circuits[record.name] = circuit;
    const grouped = new Map<string, UsedRef[]>();
    for (const placed of circuit.placed) {
      const refs = grouped.get(placed.id) ?? [];
      refs.push({ ref: placed.ref, values: placed.values });
      grouped.set(placed.id, refs);
    }
    for (const [id, refs] of grouped) {
      const list = byPart.get(id) ?? [];
      list.push({ id: record.name, title: record.title, href: at.block(record.name), refs });
      byPart.set(id, list);
    }
  }
  const boards = parts.filter((item) => isBoard(item.record));
  const uses: Record<string, PartUse> = {};
  for (const { record } of parts) {
    const onBoards: OnBoard[] = [];
    for (const board of boards) {
      if (board.record.registryDependencies.some((dependency) => dependencyId(dependency) === record.name)) {
        onBoards.push({ id: board.record.name, title: board.record.title, href: at.part(board.record.name) });
      }
    }
    uses[record.name] = { blocks: byPart.get(record.name) ?? [], boards: onBoards };
  }
  return { circuits, uses };
}

/** The index card's maker line: "Matches …", "Generic, follows …", or "Generic". */
export function matchLine(real: Real): string {
  const named = [real.maker, real.number].filter((value): value is string => value !== undefined && value !== null && value !== "").join(" ");
  if (named === "") return "Generic";
  return real.generic ? `Generic, follows ${named}` : `Matches ${named}`;
}

/** The index card's place line. A scene placement wins over a board the part only sits inside. */
export function whereLine(use: PartUse): string {
  const count = use.blocks.length;
  if (count === 1) return "in 1 Block";
  if (count > 1) return `in ${count} Blocks`;
  const board = use.boards[0];
  if (use.boards.length === 1 && board !== undefined) return `on the ${board.title}`;
  if (use.boards.length > 1) return `on ${use.boards.length} boards`;
  return "in no Block yet";
}
