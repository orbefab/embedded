// Reads the registry `orbe registry build` wrote into what the pages render from, and draws each
// part's scene with orbe-wasm's `partScene`. Runs in Node, at build time only.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { at } from "../src/paths.ts";
import { si } from "../src/format.ts";
import {
  type BlockRecord,
  dependencyId,
  type IndexRow,
  type Item,
  type ItemRecord,
  type PartRecord,
  type ScenePart,
  type Site,
  type SiteData,
} from "../src/records.ts";

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
    return {
      record: read(last.url) as ItemRecord,
      listed: last.listed !== false,
    };
  });
  const parts: Item<PartRecord>[] = [];
  const blocks: Item<BlockRecord>[] = [];
  for (const { record, listed } of newest) {
    if (record.type === "part") parts.push({ record, listed });
    else blocks.push({ record, listed });
  }
  return { name: index.name, parts, blocks, bytes: (sha256) => readFileSync(join(dir, "files", sha256)) };
}

/** A Block file a page can show as text: the scene, the tests, the firmware's sources and the
 * README. */
export const isText = (path: string) =>
  path === "scene.jsonc" || path === "README.md" || path.startsWith("tests/") || /^firmware\/.+\.(ino|c|cpp|h)$/.test(path);

export function siteData(
  catalog: Catalog,
  site: Site,
  shown: Readonly<Record<string, string>>,
  scenes: Readonly<Record<string, readonly ScenePart[]>>,
): SiteData {
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const texts: Record<string, string> = {};
  for (const { record } of catalog.blocks) {
    for (const file of record.files) if (isText(file.path)) texts[file.sha256] = decoder.decode(catalog.bytes(file.sha256));
  }
  return { site, parts: catalog.parts, blocks: catalog.blocks, texts, scenes, shown };
}

/** Reads one Block's placed parts. `files` is `[path, bytes]`: the Block's own files, and every
 * catalog part's `part.jsonc` at `parts/<id>/part.jsonc`. */
export type ReadScene = (files: readonly (readonly [string, Uint8Array])[]) => string;

/** Each Block's placed parts, from the scene reader. A finding fails the build. */
export function blockScenes(catalog: Catalog, read: ReadScene): Record<string, ScenePart[]> {
  const partFiles: [string, Uint8Array][] = [];
  for (const { record } of catalog.parts) {
    const file = record.files.find((entry) => entry.path === "part.jsonc");
    if (file !== undefined) partFiles.push([`parts/${record.name}/part.jsonc`, catalog.bytes(file.sha256)]);
  }
  const scenes: Record<string, ScenePart[]> = {};
  for (const { record } of catalog.blocks) {
    const files: [string, Uint8Array][] = [...partFiles];
    for (const file of record.files) files.push([file.path, catalog.bytes(file.sha256)]);
    try {
      scenes[record.name] = JSON.parse(read(files)) as ScenePart[];
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      throw new Error(`${record.name}: ${message}`);
    }
  }
  return scenes;
}

/** Draws one part on its own: orbe-wasm's `partScene` on the part's files and those of every
 * part it is built from, each as `[parts/<id>/<path>, bytes]`. */
export type DrawPart = (id: string, files: readonly (readonly [string, Uint8Array])[]) => string;

interface DrawnScene {
  instances: { params?: Record<string, number | string>; part: string }[];
  parts: Record<string, { view?: { recolour?: readonly { by?: string }[] } }>;
}

/** Sets each required param to its example. The caption names a value a recolour draws. */
function supplyPartScene(sceneText: string, record: PartRecord): { scene: string; shown?: string } {
  const scene = JSON.parse(sceneText) as DrawnScene;
  const root = scene.instances[0];
  if (root === undefined) throw new Error(`${record.name}: the part scene has no instance`);
  root.params ??= {};
  const named = new Set<string>();
  for (const entry of scene.parts[root.part]?.view?.recolour ?? []) if (typeof entry.by === "string") named.add(entry.by);
  const labels: string[] = [];
  for (const param of record.params) {
    if (param.required !== true || param.default !== undefined || param.example === undefined) continue;
    if (root.params[param.name] !== undefined) continue;
    root.params[param.name] = param.example;
    if (!named.has(param.name)) continue;
    labels.push(typeof param.example === "number" ? si(param.example, param.unit) : param.example);
  }
  return { scene: JSON.stringify(scene), shown: labels.length === 0 ? undefined : `Shown at ${labels.join(" and ")}.` };
}

/** Each part's scene message, by part id, with each model pointed at its file in the registry,
 * and the sentence naming an example a recolour draws. */
export function partViews(catalog: Catalog, draw: DrawPart): Map<string, { scene: string; shown?: string }> {
  const byName = new Map(catalog.parts.map((item) => [item.record.name, item.record]));
  const closure = (name: string, seen = new Set<string>()): Set<string> => {
    if (seen.has(name)) return seen;
    seen.add(name);
    for (const dependency of byName.get(name)?.registryDependencies ?? []) closure(dependencyId(dependency), seen);
    return seen;
  };
  const views = new Map<string, { scene: string; shown?: string }>();
  for (const { record } of catalog.parts) {
    const files: [string, Uint8Array][] = [];
    const urls = new Map<string, string>();
    for (const name of closure(record.name)) {
      for (const file of byName.get(name)?.files ?? []) {
        files.push([`parts/${name}/${file.path}`, catalog.bytes(file.sha256)]);
        urls.set(`/parts/${name}/${file.path}`, at.file(file.sha256));
      }
    }
    const scene = JSON.parse(draw(record.name, files)) as { parts: Record<string, { model: string | null }> };
    for (const part of Object.values(scene.parts)) {
      if (part.model !== null) {
        const url = urls.get(part.model);
        if (url === undefined) throw new Error(`${record.name}'s view names ${part.model}, which no record lists`);
        part.model = url;
      }
    }
    views.set(record.name, supplyPartScene(JSON.stringify(scene), record));
  }
  return views;
}
