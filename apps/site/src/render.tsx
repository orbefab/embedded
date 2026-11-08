// What build.mjs runs on the server side: every page as HTML, every part's view, and the
// static host's headers, all from one registry build.
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";
import type { Catalog } from "./catalog";
import { BlockPage, BlocksPage } from "./pages/blocks";
import { HomePage, NotFoundPage } from "./pages/home";
import { Shell, type Site } from "./pages/layout";
import { matches, PartPage } from "./pages/part";
import { PartsPage } from "./pages/parts";
import { at, BASE } from "./paths";
import { dependencyId } from "./records";

export { readCatalog } from "./catalog";
export type { Site } from "./pages/layout";

/** The built client: its script and its stylesheets, as URLs. */
export interface Assets {
  readonly script: string;
  readonly styles: readonly string[];
}

/** A file to write, at its path in the output folder. */
export interface Output {
  readonly file: string;
  readonly text: string;
}

interface Page {
  /** The URL path, `/embedded/...`, ending in `/`, or the 404 page's file. */
  readonly path: string;
  readonly title: string;
  readonly description: string;
  readonly body: ReactNode;
}

const fileOf = (path: string) => (path.endsWith("/") ? `${path.slice(1)}index.html` : path.slice(1));

function html(page: Page, site: Site, assets: Assets): string {
  const main = renderToString(<Shell site={site}>{page.body}</Shell>);
  const title = page.path === at.home ? page.title : `${page.title} | orbe`;
  const url = `https://orbefab.com${page.path.endsWith("/") ? page.path : BASE}`;
  const head = renderToString(
    <>
      <meta charSet="utf-8" />
      <meta content="width=device-width, initial-scale=1, viewport-fit=cover" name="viewport" />
      <title>{title}</title>
      <meta content={page.description} name="description" />
      <meta content="#111111" name="theme-color" />
      <meta content="website" property="og:type" />
      <meta content="Orbefab" property="og:site_name" />
      <meta content={title} property="og:title" />
      <meta content={page.description} property="og:description" />
      <meta content={url} property="og:url" />
      <meta content={`https://orbefab.com${at.asset("logo.png")}`} property="og:image" />
      <link href={url} rel="canonical" />
      <link href={at.asset("favicon.ico")} rel="icon" sizes="any" />
      <link href={at.asset("favicon.svg")} rel="icon" type="image/svg+xml" />
      <link href={at.asset("apple-touch-icon.png")} rel="apple-touch-icon" sizes="180x180" />
      {assets.styles.map((href) => (
        <link href={href} key={href} rel="stylesheet" />
      ))}
      {main.includes("data-island=") ? <script src={assets.script} type="module" /> : null}
    </>
  );
  return `<!doctype html><html lang="en" class="dark"><head>${head}</head><body>${main}</body></html>\n`;
}

/** Every page of the site. */
export function renderPages(catalog: Catalog, site: Site, assets: Assets): Output[] {
  const pages: Page[] = [
    {
      path: at.home,
      title: "orbe, a hardware simulator for AI agents",
      description: "Describe a circuit as a scene file, flash it with real firmware, and drive it from the command line. Blocks play live in 3D here.",
      body: <HomePage catalog={catalog} site={site} />,
    },
    {
      path: at.parts,
      title: "Parts",
      description: `The ${catalog.parts.length} parts of the ${catalog.name} registry, each with its pins, ratings, 3D model and files.`,
      body: <PartsPage catalog={catalog} />,
    },
    ...catalog.parts.map((item) => ({
      path: at.part(item.record.name),
      title: item.record.title,
      description: item.record.description ?? `${matches(item.record)}.`,
      body: <PartPage catalog={catalog} item={item} site={site} />,
    })),
    {
      path: at.blocks,
      title: "Blocks",
      description: "Small circuits that work, each with its scene, firmware and tests, live in 3D in the browser.",
      body: <BlocksPage catalog={catalog} />,
    },
    ...catalog.blocks.map((item) => ({
      path: at.block(item.record.name),
      title: item.record.title,
      description: item.record.description ?? item.record.title,
      body: <BlockPage catalog={catalog} item={item} site={site} />,
    })),
    { path: `${BASE}404.html`, title: "Not here", description: "Nothing is at this address.", body: <NotFoundPage /> },
  ];
  return pages.map((page) => ({ file: fileOf(page.path), text: html(page, site, assets) }));
}

/** Draws one part on its own: orbe-wasm's `partScene` on the part's files and those of every
 * part it is built from, each as `[parts/<id>/<path>, bytes]`. */
export type DrawPart = (id: string, files: readonly (readonly [string, Uint8Array])[]) => string;

/** Each part's scene message, with each model pointed at its file in the registry. */
export function partViews(catalog: Catalog, draw: DrawPart): Output[] {
  const byName = new Map(catalog.parts.map((item) => [item.record.name, item.record]));
  const closure = (name: string, seen = new Set<string>()): Set<string> => {
    if (seen.has(name)) return seen;
    seen.add(name);
    for (const dependency of byName.get(name)?.registryDependencies ?? []) closure(dependencyId(dependency), seen);
    return seen;
  };
  return catalog.parts.map(({ record }) => {
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
    return { file: fileOf(at.partView(record.name)), text: JSON.stringify(scene) };
  });
}

/** The static host's headers. Hashed files never change, so they cache for a year. Every
 * other file revalidates, since it keeps its URL across deploys. */
export const headers = `${BASE}*
  X-Content-Type-Options: nosniff
${BASE}assets/*
  Cache-Control: public, max-age=31536000, immutable
${BASE}r/*
  Access-Control-Allow-Origin: *
${BASE}r/files/*
  Cache-Control: public, max-age=31536000, immutable
${BASE}fonts/*
  Cache-Control: public, max-age=2592000
`;
