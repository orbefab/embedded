// The Vite plugin that feeds the site what scripts/site.sh built from this commit: the registry
// in .scratch/site/registry, orbe-wasm in .scratch/site/wasm and the AVR runtime's source in
// .scratch/site/sources. It checks the brand copy, then writes the public folder Vite serves and
// copies under /orbe/, and gives the server side the catalog as `virtual:orbe-site`.
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import type { Plugin } from "vite";
import { BASE } from "../src/paths.ts";
import type { SiteData } from "../src/records.ts";
import { blockScenes, type Catalog, partViews, readCatalog, siteData } from "./catalog.ts";

const REPO = resolve(import.meta.dirname, "../../..");
const INPUT = join(REPO, ".scratch/site");
/** The folder Vite serves at the base and copies into the build: the brand's public files, the
 * registry at r/, orbe-wasm at orbe-wasm/, each part's view.json, the runtime source and
 * _headers. When install lines show, it also holds install.sh, served at /orbe/install.sh. */
export const PUBLIC = join(INPUT, "public");
const MODULE = "virtual:orbe-site";

/** Workers static assets read `_headers` at the top of the assets folder, with public URL paths.
 * Hashed build files keep for a year. Hashed registry files keep for a year too, set on the
 * 200 by the Worker, since a rule here would also reach a 404 at the same path. The index and
 * the schemas keep their default, a revalidation each use against their ETag. */
const HEADERS = `${BASE}*
  X-Content-Type-Options: nosniff
${BASE}assets/*
  Cache-Control: public, max-age=31536000, immutable
${BASE}r/*
  Access-Control-Allow-Origin: *
${BASE}fonts/*
  Cache-Control: public, max-age=2592000
`;

function fail(message: string): never {
  throw new Error(`site: ${message}`);
}

function xmlText(text: string): string {
  return text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

/** Absolute page URLs for https://orbefab.com/orbe/sitemap.xml. Embed routes stay out. */
export function sitemapXml(catalog: Catalog): string {
  const paths = [
    "/orbe/",
    "/orbe/blocks/",
    "/orbe/parts/",
    ...catalog.blocks.map((item) => item.record.name).sort().map((id) => `/orbe/blocks/${id}/`),
    ...catalog.parts.map((item) => item.record.name).sort().map((id) => `/orbe/parts/${id}/`),
  ];
  const body = paths.map((path) => `  <url><loc>${xmlText(`https://orbefab.com${path}`)}</loc></url>`).join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${body}\n</urlset>\n`;
}

async function assemble(install: boolean): Promise<SiteData> {
  execFileSync(join(REPO, "scripts/brand.sh"), ["check"], { stdio: "inherit" });
  const registry = join(INPUT, "registry");
  const wasm = join(INPUT, "wasm");
  const sources = join(INPUT, "sources");
  for (const need of [join(registry, "index.json"), join(wasm, "package.json"), sources]) {
    if (!existsSync(need)) fail(`${need} is missing. scripts/site.sh builds the registry, orbe-wasm and the sources first.`);
  }
  const engine = JSON.parse(readFileSync(join(wasm, "package.json"), "utf8")) as {
    version: string;
    files: string[];
    orbe?: { commit?: string | null };
  };
  const orbeWasm = (await import(pathToFileURL(join(wasm, "orbe_wasm.js")).href)) as {
    initSync(options: { module: Uint8Array }): void;
    Block: new () => { add(path: string, bytes: Uint8Array): void };
    partScene(id: string, block: unknown): string;
    sceneParts(block: unknown): string;
  };
  orbeWasm.initSync({ module: readFileSync(join(wasm, "orbe_wasm_bg.wasm")) });

  const catalog = readCatalog(registry);
  const open = (files: readonly (readonly [string, Uint8Array])[]) => {
    const block = new orbeWasm.Block();
    for (const [path, bytes] of files) block.add(path, bytes);
    return block;
  };
  const views = partViews(catalog, (id, files) => orbeWasm.partScene(id, open(files)));
  const scenes = blockScenes(catalog, (files) => orbeWasm.sceneParts(open(files)));

  rmSync(PUBLIC, { recursive: true, force: true });
  cpSync(join(REPO, "packages/brand/website/public"), PUBLIC, { recursive: true });
  cpSync(join(REPO, "packages/brand/OFL.txt"), join(PUBLIC, "fonts/OFL.txt"));
  cpSync(registry, join(PUBLIC, "r"), { recursive: true });
  for (const file of engine.files) cpSync(join(wasm, file), join(PUBLIC, "orbe-wasm", file));
  cpSync(sources, join(PUBLIC, "sources"), { recursive: true });
  const shown: Record<string, string> = {};
  for (const [id, view] of views) {
    mkdirSync(join(PUBLIC, "parts", id), { recursive: true });
    writeFileSync(join(PUBLIC, "parts", id, "view.json"), view.scene);
    if (view.shown !== undefined) shown[id] = view.shown;
  }
  if (install) cpSync(join(REPO, "scripts/install.sh"), join(PUBLIC, "install.sh"));
  writeFileSync(join(PUBLIC, "_headers"), HEADERS);
  writeFileSync(join(PUBLIC, "sitemap.xml"), sitemapXml(catalog));

  const commit = engine.orbe?.commit ?? null;
  console.log(`site: ${catalog.parts.length} parts and ${catalog.blocks.length} Blocks from ${catalog.name}, sources ${readdirSync(sources).join(", ")}`);
  console.log(`site: install lines ${install ? "shown" : "hidden, the CLI is coming"}; orbe-wasm ${engine.version} from ${commit ?? "no commit"}`);
  if (commit === null || commit.endsWith("-dirty")) console.warn("site: warning: orbe-wasm was not built from a clean commit. Deploy a build of a clean checkout.");
  return siteData(catalog, { registry: catalog.name, engine: { version: engine.version, commit } }, shown, scenes);
}

/** `install` is ORBE_SITE_INSTALL=1: install lines show. The footer links the repo either way. */
export function siteInputs(install: boolean): Plugin {
  let data: SiteData | null = null;
  return {
    name: "orbe-site-inputs",
    async config(_, env) {
      if (env.isPreview) return;
      // A build resolves this config once per environment, so it reads the inputs once and
      // shares them. A dev server reads them again on each restart.
      const once = globalThis as { orbeSiteInputs?: Promise<SiteData> };
      data = await (env.command === "build" ? (once.orbeSiteInputs ??= assemble(install)) : assemble(install));
      return { define: { __ORBE_SITE_INSTALL__: JSON.stringify(install) } };
    },
    resolveId: (id) => (id === MODULE ? `\0${MODULE}` : undefined),
    load(id) {
      if (id !== `\0${MODULE}`) return undefined;
      if (data === null) fail(`${MODULE} was imported before the inputs were read`);
      return `export default ${JSON.stringify(data)};`;
    },
  };
}
