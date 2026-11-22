// The registry records docs/registry.md describes, as the pages read them. The build reads them
// from the registry `orbe registry build` wrote from this same commit.

export interface FileEntry {
  readonly path: string;
  readonly url: string;
  readonly sha256: string;
  readonly bytes: number;
  readonly license: string;
}

interface Common {
  readonly format: 1;
  readonly name: string;
  readonly version: string;
  readonly title: string;
  readonly description?: string;
  readonly categories?: readonly string[];
  readonly license: string;
  readonly registryDependencies: readonly string[];
  readonly files: readonly FileEntry[];
  readonly hash: string;
}

/** Where a value comes from: a datasheet's page, a closed form, or the user's own scene. */
export type Source =
  | { readonly datasheet: { readonly doc: string; readonly at: string; readonly basis: string } }
  | { readonly closed_form: { readonly note: string } }
  | "user";

export interface Real {
  readonly maker?: string | null;
  readonly number?: string | null;
  readonly datasheet?: string | null;
  readonly generic?: boolean;
}

export interface Pin {
  readonly name: string;
  readonly leads?: readonly number[];
  readonly refs?: readonly string[];
}

export interface Choice {
  readonly name: string;
  readonly number?: string;
  readonly tint?: string;
}

export interface Param {
  readonly name: string;
  readonly unit?: string;
  readonly required?: boolean;
  readonly default?: number | string | null;
  readonly example?: number | string;
  readonly above?: number;
  readonly below?: number;
  readonly choices?: readonly Choice[];
  readonly file?: string;
}

export interface Rating {
  readonly name: string;
  readonly value: number | null;
  readonly unit?: string;
  readonly source?: Source;
}

export interface PartRecord extends Common {
  readonly type: "part";
  readonly real: Real;
  readonly pins: readonly Pin[];
  readonly params: readonly Param[];
  readonly ratings: readonly Rating[];
  readonly notSimulated: readonly { readonly what: string; readonly why: string }[];
  readonly docs?: readonly { readonly name: string; readonly url?: string }[];
  readonly buy?: readonly { readonly seller: string; readonly url: string; readonly checked: string }[];
  readonly lit?: { readonly of: string; readonly above: number; readonly unit: string };
  readonly dim?: { readonly of: string; readonly above: number; readonly unit: string };
  readonly pressable?: true;
  /** A board's holes, such as a breadboard's. Each hole is also one of `pins`. */
  readonly holes?: { readonly depth: number; readonly strips: readonly (readonly string[])[] };
}

export interface BlockRecord extends Common {
  readonly type: "block";
  readonly board: string;
}

export type ItemRecord = PartRecord | BlockRecord;

export interface IndexRow {
  readonly name: string;
  readonly type: "part" | "block";
  readonly version: string;
  readonly url: string;
  /** Absent or true when search and the parts list offer this part. */
  readonly listed?: boolean;
}

/** A dependency's id, from `<id>@<version>`. */
export const dependencyId = (dependency: string) => dependency.slice(0, dependency.lastIndexOf("@"));

/** One item as its pages show it: its newest record, every version the registry serves, and
 * the items whose newest record names it. */
export interface Item<R extends ItemRecord> {
  readonly record: R;
  readonly versions: readonly string[];
  readonly usedBy: readonly string[];
  /** False when search and the parts list leave this part out. Absent means they offer it. */
  readonly listed?: boolean;
}

/** What every page knows of the build. */
export interface Site {
  /** The registry's name, such as `@orbefab`. */
  readonly registry: string;
  /** orbe-wasm's version and the commit it was built from. */
  readonly engine: { readonly version: string; readonly commit: string | null };
}

/** One value a scene writes on a part, as the scene reader returns it. */
export interface SceneValue {
  readonly name: string;
  /** A number, in the param's SI unit. A duration is seconds. */
  readonly amount?: number;
  /** The param's unit, such as `ohm` or `1`. */
  readonly unit?: string;
  /** A choice, as written. */
  readonly text?: string;
  /** A file param's path, as written. */
  readonly file?: string;
}

/** One part a scene places, in the scene's order. */
export interface ScenePart {
  readonly ref: string;
  /** The catalog id. */
  readonly part: string;
  /** The part file has a press input. A child of this part does not count. */
  readonly press: boolean;
  readonly values: readonly SceneValue[];
}

/** Everything the pages render from, as the build reads it from one registry build. `texts`
 * holds each Block text file a page can show, by its SHA-256. `scenes` is each Block's placed
 * parts, from the scene reader. */
export interface SiteData {
  readonly site: Site;
  readonly parts: readonly Item<PartRecord>[];
  readonly blocks: readonly Item<BlockRecord>[];
  readonly texts: Readonly<Record<string, string>>;
  readonly scenes: Readonly<Record<string, readonly ScenePart[]>>;
  /** A sentence such as "Shown at 220 Ω." for a part page whose view draws a supplied value. */
  readonly shown: Readonly<Record<string, string>>;
}
