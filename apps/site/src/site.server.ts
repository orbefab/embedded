// What each page renders, read from the catalog the build gave the server side. Only the server
// functions in data.ts call these, so none of it reaches the browser.
import data from "virtual:orbe-site";
import { at } from "./paths";
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
  readonly board: string;
  readonly parts: number;
}

/** A part as the parts list shows it and the search reads it. */
export interface Listed extends Searchable {
  readonly href: string;
}

export const site = () => data.site;

function linked(names: readonly string[]): Record<string, Linked> {
  const entry = (name: string): Linked => {
    const block = data.blocks.find((item) => item.record.name === name);
    if (block !== undefined) return { title: block.record.title, href: at.block(name) };
    return { title: data.parts.find((item) => item.record.name === name)?.record.title ?? name, href: at.part(name) };
  };
  return Object.fromEntries(names.map((name) => [name, entry(name)]));
}

const card = ({ record }: Item<BlockRecord>): BlockCard => ({
  name: record.name,
  title: record.title,
  description: record.description,
  board: record.board,
  parts: record.registryDependencies.length,
});

const dependencyIds = (record: PartRecord | BlockRecord) => record.registryDependencies.map((dependency) => dependency.slice(0, dependency.lastIndexOf("@")));

export const blockCards = () => data.blocks.map(card);

export const home = () => ({
  blocks: blockCards(),
  boards: data.parts.filter((item) => item.record.categories?.includes("board")).map(({ record }) => ({ title: record.title, href: at.part(record.name) })),
  parts: data.parts.length,
});

export const parts = (): Listed[] =>
  [...data.parts]
    .sort((a, b) => a.record.title.localeCompare(b.record.title))
    .map(({ record }) => ({
      name: record.name,
      title: record.title,
      maker: record.real.maker,
      number: record.real.number,
      description: record.description,
      categories: record.categories,
      href: at.part(record.name),
    }));

export function part(id: string) {
  const item = data.parts.find((entry) => entry.record.name === id);
  if (item === undefined) return null;
  return { item, links: linked([...dependencyIds(item.record), ...item.usedBy]) };
}

/** A Block's text files the page shows: the scene, the firmware's sources and the tests, and
 * the README when its install lines can show. */
export function block(id: string) {
  const item = data.blocks.find((entry) => entry.record.name === id);
  if (item === undefined) return null;
  const texts = item.record.files
    .filter((file) => file.path !== "README.md" || __ORBE_SITE_INSTALL__)
    .flatMap((file) => {
      const text = data.texts[file.sha256];
      return text === undefined ? [] : [{ path: file.path, text }];
    });
  return { item, links: linked(dependencyIds(item.record)), texts };
}
