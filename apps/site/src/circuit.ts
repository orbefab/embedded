// A Block's parts and a part's "used in" list, both from the registry and the scene files.
// The card count and "Parts in this circuit" are both `placed.length`. A reference has no
// role here: the registry does not record one.

import { si } from "./format";
import { parseJsonc } from "./jsonc";
import { at } from "./paths";
import { dependencyId, type BlockRecord, type Item, type Param, type PartRecord } from "./records";

/** One reference a Block's scene places. */
export interface PlacedPart {
  readonly ref: string;
  readonly id: string;
  /** The scene's params, in the scene's order, already in words (`220 Ω`, `red`, `fade.ino.hex`). */
  readonly values: readonly string[];
}

/** The board a Block runs on. */
export interface BoardFold {
  readonly id: string;
  readonly title: string;
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
  /** `tests/*.test` files. `orbe test` reports one result per file. */
  readonly tests: number;
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
  // A file param is a path (`build/fade.ino.hex`). A value such as `1/4` is the value itself.
  if (param?.file !== undefined && text.includes("/")) return text.slice(text.lastIndexOf("/") + 1);
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

function sceneError(name: string, detail: string): Error {
  return new Error(`${name} scene.jsonc ${detail}`);
}

function placedParts(record: BlockRecord, texts: Readonly<Record<string, string>>, parts: readonly Item<PartRecord>[]): readonly PlacedPart[] {
  const file = record.files.find((entry) => entry.path === "scene.jsonc");
  const text = file === undefined ? undefined : texts[file.sha256];
  if (text === undefined) throw new Error(`${record.name} scene.jsonc is missing`);
  let scene: unknown;
  try {
    scene = parseJsonc(text);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`${record.name} scene.jsonc: ${message}`);
  }
  const raw = scene !== null && typeof scene === "object" && "parts" in scene ? scene.parts : undefined;
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) throw sceneError(record.name, "parts is not an object");
  const placed = Object.entries(raw).map(([ref, body]) => {
    if (body === null || typeof body !== "object" || Array.isArray(body)) throw sceneError(record.name, `part ${ref} is not an object`);
    const fields = body as Record<string, unknown>;
    const id = fields.type;
    if (typeof id !== "string") throw sceneError(record.name, `part ${ref} has no type`);
    return { ref, id, values: valueTexts(id, fields, parts) };
  });
  if (!placed.some((part) => part.id === record.board)) throw sceneError(record.name, `does not place its board ${record.board}`);
  return placed;
}

function boardFold(id: string, parts: readonly Item<PartRecord>[]): BoardFold {
  const record = parts.find((item) => item.record.name === id)?.record;
  if (record === undefined) throw new Error(`board ${id} is not a part`);
  return { id, title: record.title };
}

function sourceGroup(path: string): SourceFile["group"] | undefined {
  if (path === "scene.jsonc") return "Scene";
  if (path.startsWith("tests/")) return "Test";
  if (/^firmware\/.+\.(?:ino|c|cpp|h)$/.test(path)) return "Sketch";
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

/** One result per `tests/*.test` file, which is what `orbe test` counts. */
function testCount(record: BlockRecord): number {
  return record.files.filter((file) => file.path.startsWith("tests/") && file.path.endsWith(".test")).length;
}

function circuitOf(record: BlockRecord, texts: Readonly<Record<string, string>>, parts: readonly Item<PartRecord>[]): BlockCircuit {
  return {
    placed: placedParts(record, texts, parts),
    board: boardFold(record.board, parts),
    tests: testCount(record),
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

/** The glance row. The label keeps "used in" or "on boards", and the value does not repeat it.
 * Nothing, when the part is in no Block and on no board. */
export function glanceLine(use: PartUse): { readonly label: string; readonly value: string; readonly href: string | undefined } | undefined {
  const count = use.blocks.length;
  if (count === 1) return { label: "used in", value: "1 Block", href: "#used" };
  if (count > 1) return { label: "used in", value: `${count} Blocks`, href: "#used" };
  const board = use.boards[0];
  if (use.boards.length === 1 && board !== undefined) return { label: "on boards", value: board.title, href: "#boards" };
  if (use.boards.length > 1) return { label: "on boards", value: `${use.boards.length} boards`, href: "#boards" };
  return undefined;
}

/** The index card's place line. A scene placement wins over a board the part only sits inside.
 * Empty when the part is in no Block and on no board. */
export function whereLine(use: PartUse): string {
  const count = use.blocks.length;
  if (count === 1) return "in 1 Block";
  if (count > 1) return `in ${count} Blocks`;
  const board = use.boards[0];
  if (use.boards.length === 1 && board !== undefined) return `on the ${board.title}`;
  if (use.boards.length > 1) return `on ${use.boards.length} boards`;
  return "";
}
