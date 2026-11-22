// A Block's parts and a part's "used in" list. The parts and the values written on them come
// from the scene reader. The card count and "Parts in this circuit" are both `placed.length`.

import { si } from "./format";
import { at } from "./paths";
import { dependencyId, type BlockRecord, type Item, type PartRecord, type ScenePart, type SceneValue } from "./records";

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
  /** The first placed part with a press input, as `@ref`. Null when the scene has none. */
  readonly hold: `@${string}` | null;
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

function showOne(value: SceneValue): string {
  if (value.file !== undefined) {
    const slash = value.file.lastIndexOf("/");
    return slash < 0 ? value.file : value.file.slice(slash + 1);
  }
  if (value.text !== undefined) return value.text;
  if (value.amount !== undefined) return si(value.amount, value.unit ?? "1");
  return "";
}

/** The scene's values in words. A file is its name. The first value is bare, and each later one
 * is prefixed with its name, so a trimmer reads `10 kΩ, position 0.5`. */
function showValues(values: readonly SceneValue[]): readonly string[] {
  const shown = values.flatMap((value) => {
    const text = showOne(value);
    return text === "" ? [] : [{ name: value.name, text }];
  });
  if (shown.length <= 1) return shown.map((entry) => entry.text);
  return shown.map((entry, index) => (index === 0 ? entry.text : `${entry.name} ${entry.text}`));
}

function placedParts(record: BlockRecord, scene: readonly ScenePart[]): readonly PlacedPart[] {
  const placed = scene.map((part) => ({ ref: part.ref, id: part.part, values: showValues(part.values) }));
  if (!placed.some((part) => part.id === record.board)) {
    throw new Error(`${record.name} scene.jsonc does not place its board ${record.board}`);
  }
  return placed;
}

/** The first placed part with a press input, in the scene's order. */
function holdOf(scene: readonly ScenePart[]): `@${string}` | null {
  const found = scene.find((part) => part.press);
  return found === undefined ? null : `@${found.ref}`;
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

function circuitOf(
  record: BlockRecord,
  texts: Readonly<Record<string, string>>,
  parts: readonly Item<PartRecord>[],
  scene: readonly ScenePart[] | undefined,
): BlockCircuit {
  if (scene === undefined) throw new Error(`${record.name} scene.jsonc is missing`);
  return {
    placed: placedParts(record, scene),
    hold: holdOf(scene),
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
  scenes: Readonly<Record<string, readonly ScenePart[]>>,
): { readonly circuits: Readonly<Record<string, BlockCircuit>>; readonly uses: Readonly<Record<string, PartUse>> } {
  const circuits: Record<string, BlockCircuit> = {};
  const byPart = new Map<string, UsedInBlock[]>();
  for (const { record } of blocks) {
    const circuit = circuitOf(record, texts, parts, scenes[record.name]);
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
