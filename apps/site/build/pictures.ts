// The parts list's pictures. apps/site/scripts/pictures.ts renders one per listed part from its
// 3D model into apps/site/pictures, and manifest.json there keys each picture to the models it
// was drawn from. The build copies the pictures it finds, and the check fails while a listed
// part has no picture or its key differs from the registry the site is built from.
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { dependencyId } from "../src/records.ts";
import type { Catalog } from "./catalog.ts";

export const PICTURES = resolve(import.meta.dirname, "../pictures");
export const MANIFEST = join(PICTURES, "manifest.json");
/** What a person runs to draw missing or stale pictures, after scripts/site.sh. */
export const RENDER_COMMAND = "pnpm --filter @orbe/site pictures";

export const pictureFile = (id: string) => join(PICTURES, `${id}.webp`);

/** Each listed part id, in id order. */
export function listedParts(catalog: Catalog): string[] {
  return catalog.parts
    .filter((item) => item.listed !== false)
    .map((item) => item.record.name)
    .sort();
}

/** The key a picture of `id` is rendered from: its GLB's SHA-256 as the registry records it.
 * A part drawn with the parts it is built from, such as a board, has several models, and its
 * key is the SHA-256 of their SHA-256s, sorted, one per line. Null for a part with no model. */
export function pictureKey(catalog: Catalog, id: string): string | null {
  const byName = new Map(catalog.parts.map((item) => [item.record.name, item.record]));
  const seen = new Set<string>();
  const models = new Set<string>();
  const visit = (name: string) => {
    if (seen.has(name)) return;
    seen.add(name);
    const record = byName.get(name);
    if (record === undefined) return;
    for (const file of record.files) if (file.path.endsWith(".glb")) models.add(file.sha256);
    for (const dependency of record.registryDependencies) visit(dependencyId(dependency));
  };
  visit(id);
  const sorted = [...models].sort();
  if (sorted.length === 0) return null;
  if (sorted.length === 1) return sorted[0] as string;
  return createHash("sha256").update(sorted.join("\n")).digest("hex");
}

/** Part id to the key its picture was rendered from. */
export function readManifest(): Record<string, string> {
  if (!existsSync(MANIFEST)) return {};
  return JSON.parse(readFileSync(MANIFEST, "utf8")) as Record<string, string>;
}

export function manifestText(manifest: Readonly<Record<string, string>>): string {
  const sorted = Object.fromEntries(Object.keys(manifest).sort().map((id) => [id, manifest[id]]));
  return `${JSON.stringify(sorted, null, 2)}\n`;
}

/** One line per listed part whose picture is missing or stale, and per picture of a part the
 * site does not list. Empty when every picture is current. */
export function pictureProblems(catalog: Catalog): string[] {
  const manifest = readManifest();
  const listed = listedParts(catalog);
  const problems: string[] = [];
  for (const id of listed) {
    const key = pictureKey(catalog, id);
    if (key === null) continue;
    const had = manifest[id];
    if (had === undefined || !existsSync(pictureFile(id))) problems.push(`${id} has no picture`);
    else if (had !== key) problems.push(`${id}'s picture is stale: it was drawn from model ${had.slice(0, 12)}, and the registry has ${key.slice(0, 12)}`);
  }
  const wanted = new Set(listed);
  for (const id of Object.keys(manifest)) if (!wanted.has(id)) problems.push(`${id} has a picture, and the site does not list it`);
  return problems;
}
