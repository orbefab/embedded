// The pictures on the parts list's and the Blocks list's cards. apps/site/scripts/pictures.ts
// draws one per listed part from its 3D model, and one per Block from its scene as the Block
// runs, into apps/site/pictures, and manifest.json there keys each picture to what it was drawn
// from. The build copies the pictures it finds, and the check fails while a listed part or a
// Block has no picture or its key differs from the registry the site is built from.
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { dependencyId } from "../src/records.ts";
import type { Catalog } from "./catalog.ts";

export const PICTURES = resolve(import.meta.dirname, "../pictures");
export const MANIFEST = join(PICTURES, "manifest.json");
/** What a person runs to draw missing or stale pictures, after scripts/site.sh. */
export const RENDER_COMMAND = "pnpm --filter @orbe/site pictures";

/** A picture's name in the manifest: a part's id, or `blocks/` and a Block's id. */
export const blockPicture = (id: string) => `blocks/${id}`;
export const pictureFile = (name: string) => join(PICTURES, `${name}.webp`);

/** The moment a Block's picture shows: the simulated time in milliseconds, and the part held
 * pressed from the start, as the Block page's Hold to press holds it, or null for none. */
export interface Moment {
  readonly atMs: number;
  readonly press: `@${string}` | null;
}

/** About a second in, when a Block is doing what it is for, with nothing pressed. */
export const MOMENT: Moment = { atMs: 1000, press: null };
/** A Block whose picture shows another moment, with why. Each is still the Block as it runs. */
export const MOMENT_OF: Readonly<Record<string, Partial<Moment>>> = {
  // LiquidCrystal_I2C's init keeps the glass dark until about 1.08 s, and both lines are written
  // by 1.12 s.
  "nano-lcd1602-i2c": { atMs: 1200 },
  // MD_Parola scrolls "ORBE" to the centre of the 32 x 8 display at 600 ms.
  "nano-max7219-scroll": { atMs: 600 },
  // The LED lights only while the button is down.
  "nano-button-led": { press: "@btn1" },
  "uno-button-led": { press: "@btn1" },
};

export const momentOf = (id: string): Moment => ({ ...MOMENT, ...MOMENT_OF[id] });

/** One picture the site wants: its manifest name, the key it is drawn from, and what to draw. */
export type Wanted =
  | { readonly kind: "part"; readonly name: string; readonly id: string; readonly key: string }
  | { readonly kind: "block"; readonly name: string; readonly id: string; readonly key: string; readonly moment: Moment };

/** Each listed part id, in id order. */
export function listedParts(catalog: Catalog): string[] {
  return catalog.parts
    .filter((item) => item.listed !== false)
    .map((item) => item.record.name)
    .sort();
}

/** Each Block id the Blocks list shows, in id order. */
export function listedBlocks(catalog: Catalog): string[] {
  return catalog.blocks.map((item) => item.record.name).sort();
}

/** The key a picture of part `id` is rendered from: its GLB's SHA-256 as the registry records it.
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

/** The key a picture of Block `id` is rendered from: the version the site shows, which pins its
 * files and the version of each part it runs, the hash of those files the registry records, and
 * its moment, such as `0.3.2 sha256:… at 1000 ms holding @btn1`. The engine is not in the key, so
 * a change that makes a Block run differently redraws with `-- --all`. */
export function blockPictureKey(catalog: Catalog, id: string): string {
  const item = catalog.blocks.find((each) => each.record.name === id);
  if (item === undefined) throw new Error(`the registry has no Block ${id}`);
  const { atMs, press } = momentOf(id);
  return `${item.record.version} ${item.record.hash} at ${atMs} ms${press === null ? "" : ` holding ${press}`}`;
}

/** Every picture the site wants, parts first, each in id order. A part with no model has none. */
export function wantedPictures(catalog: Catalog): Wanted[] {
  const parts = listedParts(catalog).flatMap((id): Wanted[] => {
    const key = pictureKey(catalog, id);
    return key === null ? [] : [{ kind: "part", name: id, id, key }];
  });
  const blocks = listedBlocks(catalog).map(
    (id): Wanted => ({ kind: "block", name: blockPicture(id), id, key: blockPictureKey(catalog, id), moment: momentOf(id) }),
  );
  return [...parts, ...blocks];
}

/** Picture name to the key it was rendered from. */
export function readManifest(): Record<string, string> {
  if (!existsSync(MANIFEST)) return {};
  return JSON.parse(readFileSync(MANIFEST, "utf8")) as Record<string, string>;
}

export function manifestText(manifest: Readonly<Record<string, string>>): string {
  const sorted = Object.fromEntries(Object.keys(manifest).sort().map((name) => [name, manifest[name]]));
  return `${JSON.stringify(sorted, null, 2)}\n`;
}

/** One line per wanted picture that is missing or stale, and per picture of a part or Block the
 * site does not list. Empty when every picture is current. */
export function pictureProblems(catalog: Catalog): string[] {
  const manifest = readManifest();
  const wanted = wantedPictures(catalog);
  const problems: string[] = [];
  for (const { kind, name, id, key } of wanted) {
    const had = manifest[name];
    const what = kind === "part" ? id : `Block ${id}`;
    if (had === undefined || !existsSync(pictureFile(name))) problems.push(`${what} has no picture`);
    else if (had !== key)
      problems.push(
        kind === "part"
          ? `${id}'s picture is stale: it was drawn from model ${had.slice(0, 12)}, and the registry has ${key.slice(0, 12)}`
          : `Block ${id}'s picture is stale: it was drawn from ${had}, and the registry has ${key}`,
      );
  }
  const names = new Set(wanted.map((each) => each.name));
  for (const name of Object.keys(manifest)) if (!names.has(name)) problems.push(`${name} has a picture, and the site does not list it`);
  return problems;
}
