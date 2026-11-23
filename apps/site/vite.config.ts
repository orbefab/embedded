import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { cloudflare } from "@cloudflare/vite-plugin";
import tailwindcss from "@tailwindcss/vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { PUBLIC, siteInputs } from "./build/inputs.ts";
import type { IndexRow } from "./src/records.ts";

const BRAND = resolve(import.meta.dirname, "../../packages/brand/website");
const flag = process.env.ORBE_SITE_INSTALL ?? "";
if (!["", "0", "1"].includes(flag)) {
  throw new Error(`ORBE_SITE_INSTALL is ${JSON.stringify(flag)}. Set it to 1 to show install lines, or leave it unset.`);
}

/** A path the build renders to HTML: a page, which ends in `/`, or the 404 page. The registry,
 * orbe-wasm and the other files the pages link to are copied, not rendered. */
const isPage = (path: string) =>
  path === "/orbe/404" || (path.startsWith("/orbe/") && path.endsWith("/") && !path.startsWith("/orbe/r/"));

/** The registry this build serves. A part or Block that is only in the repo has no page yet. */
const index = resolve(import.meta.dirname, "../../.scratch/site/registry/index.json");
const registryItems = existsSync(index)
  ? (JSON.parse(readFileSync(index, "utf8")) as { items: IndexRow[] }).items
  : [];
const names = (rows: readonly IndexRow[]) => [...new Set(rows.map((row) => row.name))];

/** Embed routes are unlinked, so the crawl never finds them. List each Block the registry serves. */
const embedPages = names(registryItems.filter((row) => row.type === "block")).map((name) => ({
  path: `/orbe/blocks/${name}/embed/`,
}));

/** A part whose index row has `"listed": false` is left out of the parts list, so the crawl never finds its page
 * unless a board page links it. List each one the registry serves. */
const unlistedPartPages = names(registryItems.filter((row) => row.type === "part" && row.listed === false)).map(
  (name) => ({ path: `/orbe/parts/${name}/` }),
);

/** The Worker Start renders pages in, from its server entry. */
const renderer = { name: "orbefab-embedded-render", main: "@tanstack/react-start/server-entry" };

// Start derives the router's basepath, /orbe, from `base`, and renders every page at build
// time by crawling the links from Get orbe, in a Worker built only for that. The Worker
// that deploys serves dist/client as static assets, mounted at /orbe by wrangler.jsonc's
// `assets.base_path`. redirect.js answers /embedded. `vite dev` renders each page on request
// in the prerender Worker.
export default defineConfig(({ command }) => ({
  base: "/orbe/",
  publicDir: PUBLIC,
  plugins: [
    siteInputs(flag === "1"),
    command === "build"
      ? cloudflare({ experimental: { prerenderWorker: { config: renderer, viteEnvironment: { name: "ssr" } } } })
      : cloudflare({ config: renderer, viteEnvironment: { name: "ssr" } }),
    tailwindcss(),
    tanstackStart({
      pages: [
        { path: "/orbe/" },
        { path: "/orbe/404", prerender: { outputPath: "/orbe/404.html" } },
        ...embedPages,
        ...unlistedPartPages,
      ],
      prerender: { enabled: true, crawlLinks: true, autoStaticPathsDiscovery: false, filter: ({ path }) => isPage(path) },
    }),
    react(),
  ],
  // The brand's wordmark imports `@/lib/cn`, as it does in the website repo.
  resolve: { alias: [{ find: /^@\//, replacement: `${BRAND}/src/` }] },
  // The 3D views' chunk holds three.js and loads only on a page that shows one.
  build: { chunkSizeWarningLimit: 1200 },
}));
