// What each page renders, read from the catalog the build gave the server side. Only the server
// functions in data.ts call these, so none of it reaches the browser.
import data from "virtual:orbe-site";
import { type BlockCircuit, type PartUse, readCatalogUse, whereLine } from "./circuit";
import { at } from "./paths";
import { readmeParagraphs } from "./readme";
import type { BlockRecord, Item, PartRecord } from "./records";
import type { Searchable } from "./search";

/** A link to an item's page, with its title. */
export interface Linked {
  readonly title: string;
  readonly href: string;
}

export interface BlockCard {
  readonly name: string;
  readonly title: string;
  readonly description?: string;
  /** The board's title, the same name the Block page uses. */
  readonly board: string;
  /** How many parts the scene places. The Block page's list is this long. */
  readonly parts: number;
  /** The scene places a part with a press input, so the page shows the hold control. */
  readonly button: boolean;
}

/** A part as the parts list shows it and the search reads it. */
export interface Listed extends Searchable {
  readonly href: string;
  /** "in N Blocks", "on the …", or empty when neither applies. */
  readonly use: string;
}

let catalogUse: ReturnType<typeof readCatalogUse> | undefined;

/** One pass over the scenes. The card, the Block page and every part page read this. */
function catalog() {
  catalogUse ??= readCatalogUse(data.blocks, data.parts, data.texts);
  return catalogUse;
}

function circuitOf(name: string): BlockCircuit {
  const circuit = catalog().circuits[name];
  if (circuit === undefined) throw new Error(`${name} has no scene`);
  return circuit;
}

function useOf(name: string): PartUse {
  return catalog().uses[name] ?? { blocks: [], boards: [] };
}

function linked(names: readonly string[]): Record<string, Linked> {
  const entry = (name: string): Linked => {
    const block = data.blocks.find((item) => item.record.name === name);
    if (block !== undefined) return { title: block.record.title, href: at.block(name) };
    return { title: data.parts.find((item) => item.record.name === name)?.record.title ?? name, href: at.part(name) };
  };
  return Object.fromEntries(names.map((name) => [name, entry(name)]));
}

/** A top-level scene part whose catalog record has a press input. A board's reset switch is a
 * child of the board, so it does not count. */
function sceneHasButton(circuit: BlockCircuit): boolean {
  const pressable = new Set(data.parts.filter((item) => item.record.pressable === true).map((item) => item.record.name));
  return circuit.placed.some((part) => pressable.has(part.id));
}

const card = ({ record }: Item<BlockRecord>): BlockCard => {
  const circuit = circuitOf(record.name);
  return {
    name: record.name,
    title: record.title,
    description: record.description,
    board: circuit.board.title,
    parts: circuit.placed.length,
    button: sceneHasButton(circuit),
  };
};

const dependencyIds = (record: PartRecord | BlockRecord) => record.registryDependencies.map((dependency) => dependency.slice(0, dependency.lastIndexOf("@")));

export const blockCards = () => data.blocks.map(card);

export const home = () => {
  const fade = data.blocks.find((entry) => entry.record.name === "nano-led-fade");
  const sceneFile = fade?.record.files.find((file) => file.path === "scene.jsonc");
  const scene = sceneFile === undefined ? undefined : data.texts[sceneFile.sha256];
  if (scene === undefined) throw new Error("nano-led-fade has no scene.jsonc");
  return {
    blocks: blockCards(),
    boards: data.parts.filter((item) => item.record.categories?.includes("board")).map(({ record }) => ({ title: record.title, href: at.part(record.name) })),
    parts: data.parts.length,
    scene,
  };
};

export const parts = (): Listed[] =>
  [...data.parts]
    .sort((a, b) => a.record.title.localeCompare(b.record.title))
    .map(({ record }) => {
      const use = useOf(record.name);
      return {
        name: record.name,
        title: record.title,
        maker: record.real.maker,
        number: record.real.number,
        description: record.description,
        categories: record.categories,
        href: at.part(record.name),
        use: whereLine(use),
      };
    });

export function part(id: string) {
  const item = data.parts.find((entry) => entry.record.name === id);
  if (item === undefined) return null;
  const use = useOf(id);
  const names = [...dependencyIds(item.record), ...use.blocks.map((block) => block.id), ...use.boards.map((board) => board.id)];
  return { item, links: linked(names), use, where: whereLine(use), shown: data.shown[id] };
}

/** What the embed route draws: the player, and nothing else. */
export function blockPlayer(id: string) {
  const item = data.blocks.find((entry) => entry.record.name === id);
  if (item === undefined) return null;
  return {
    name: item.record.name,
    title: item.record.title,
    description: item.record.description,
    button: sceneHasButton(circuitOf(id)),
  };
}

/** A Block's page: its scene's parts, the sketch, scene and test, and the README. The README's
 * install paragraph is kept only when install lines can show. */
export function block(id: string) {
  const item = data.blocks.find((entry) => entry.record.name === id);
  if (item === undefined) return null;
  const circuit = circuitOf(id);
  const readmeFile = item.record.files.find((file) => file.path === "README.md");
  const readmeText = readmeFile === undefined ? undefined : data.texts[readmeFile.sha256];
  if (readmeText === undefined) throw new Error(`${id} has no README.md`);
  const names = circuit.placed.map((placed) => placed.id);
  return { item, links: linked(names), circuit, readme: readmeParagraphs(readmeText, __ORBE_SITE_INSTALL__), button: sceneHasButton(circuit) };
}
