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
  readonly on?: { readonly of: string; readonly above?: number; readonly below?: number; readonly unit: string };
  readonly pressable?: true;
}

export interface BlockRecord extends Common {
  readonly type: "block";
  readonly board: string;
}

export type ItemRecord = PartRecord | BlockRecord;

export interface IndexRow {
  readonly name: string;
  readonly type: "part" | "block" | "model";
  readonly version: string;
  readonly url: string;
}

/** A dependency's id, from `<id>@<version>`. */
export const dependencyId = (dependency: string) => dependency.slice(0, dependency.lastIndexOf("@"));

/** One item as its pages show it: its newest record, every version the registry serves, and
 * the items whose newest record names it. */
export interface Item<R extends ItemRecord> {
  readonly record: R;
  readonly versions: readonly string[];
  readonly usedBy: readonly string[];
}

/** What every page knows of the build. */
export interface Site {
  /** The registry's name, such as `@orbefab`. */
  readonly registry: string;
  /** orbe-wasm's version and the commit it was built from. */
  readonly engine: { readonly version: string; readonly commit: string | null };
}

/** Everything the pages render from, as the build reads it from one registry build. `texts`
 * holds each Block text file a page can show, by its SHA-256. */
export interface SiteData {
  readonly site: Site;
  readonly parts: readonly Item<PartRecord>[];
  readonly blocks: readonly Item<BlockRecord>[];
  readonly texts: Readonly<Record<string, string>>;
}
