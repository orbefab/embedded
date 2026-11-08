import { readFileSync } from "node:fs";
import { join } from "node:path";
import { type BlockRecord, dependencyId, type IndexRow, type ItemRecord, type PartRecord } from "./records";

/** One item as its pages show it: its newest record, every version the registry serves, and
 * the items whose newest record names it. */
export interface Item<R extends ItemRecord> {
  readonly record: R;
  readonly versions: readonly string[];
  readonly usedBy: readonly string[];
}

export interface Catalog {
  readonly name: string;
  readonly parts: readonly Item<PartRecord>[];
  readonly blocks: readonly Item<BlockRecord>[];
  /** Each file's bytes, by SHA-256, from the registry's `files/`. */
  readonly bytes: (sha256: string) => Uint8Array;
}

const compareVersions = (a: string, b: string) => {
  const [x, y] = [a.split(".").map(Number), b.split(".").map(Number)];
  for (let i = 0; i < 3; i += 1) if (x[i] !== y[i]) return (x[i] ?? 0) - (y[i] ?? 0);
  return 0;
};

/** The catalog in a registry folder `orbe registry build` wrote. */
export function readCatalog(dir: string): Catalog {
  const read = (path: string) => JSON.parse(readFileSync(join(dir, path), "utf8")) as unknown;
  const index = read("index.json") as { format: number; name: string; items: readonly IndexRow[] };
  if (index.format !== 1) throw new Error(`${dir}/index.json is format ${index.format}, and the site reads 1`);
  const rows = new Map<string, IndexRow[]>();
  for (const row of index.items) rows.set(row.name, [...(rows.get(row.name) ?? []), row]);
  const newest = [...rows.values()].map((list) => {
    const sorted = [...list].sort((a, b) => compareVersions(a.version, b.version));
    const last = sorted[sorted.length - 1] as IndexRow;
    return { record: read(last.url) as ItemRecord, versions: sorted.map((row) => row.version) };
  });
  const usedBy = (name: string) =>
    newest
      .filter(({ record }) => record.registryDependencies.some((dependency) => dependencyId(dependency) === name))
      .map(({ record }) => record.name)
      .sort();
  const item = <R extends ItemRecord>(record: R, versions: readonly string[]): Item<R> => ({ record, versions, usedBy: usedBy(record.name) });
  const parts: Item<PartRecord>[] = [];
  const blocks: Item<BlockRecord>[] = [];
  for (const { record, versions } of newest) {
    if (record.type === "part") parts.push(item(record, versions));
    else blocks.push(item(record, versions));
  }
  return {
    name: index.name,
    parts,
    blocks,
    bytes: (sha256) => readFileSync(join(dir, "files", sha256)),
  };
}
