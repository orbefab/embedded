import { resolve } from "node:path";
import { cloudflare } from "@cloudflare/vite-plugin";
import tailwindcss from "@tailwindcss/vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { PUBLIC, siteInputs } from "./build/inputs.ts";

const BRAND = resolve(import.meta.dirname, "../../packages/brand/website");
const flag = process.env.ORBE_SITE_INSTALL ?? "";
if (!["", "0", "1"].includes(flag)) {
  throw new Error(`ORBE_SITE_INSTALL is ${JSON.stringify(flag)}. Set it to 1 to show install lines, or leave it unset.`);
}

/** A path the build renders to HTML: a page, which ends in `/`, or the 404 page. The registry,
 * orbe-wasm and the other files the pages link to are copied, not rendered. */
const isPage = (path: string) => path === "/embedded/404" || (path.endsWith("/") && !path.startsWith("/embedded/r/"));

/** The Worker Start renders pages in, from its server entry. */
const renderer = { name: "orbefab-embedded-render", main: "@tanstack/react-start/server-entry" };

// Start derives the router's basepath, /embedded, from `base`, and renders every page at build
// time by crawling the links from the landing page, in a Worker built only for that. The Worker
// that deploys serves dist/client as static assets, mounted at /embedded by wrangler.jsonc's
// `assets.base_path`, and runs no code. `vite dev` renders each page on request in that Worker.
export default defineConfig(({ command }) => ({
  base: "/embedded/",
  publicDir: PUBLIC,
  plugins: [
    siteInputs(flag === "1"),
    command === "build"
      ? cloudflare({ experimental: { prerenderWorker: { config: renderer, viteEnvironment: { name: "ssr" } } } })
      : cloudflare({ config: renderer, viteEnvironment: { name: "ssr" } }),
    tailwindcss(),
    tanstackStart({
      pages: [{ path: "/embedded/" }, { path: "/embedded/404", prerender: { outputPath: "/embedded/404.html" } }],
      prerender: { enabled: true, crawlLinks: true, autoStaticPathsDiscovery: false, filter: ({ path }) => isPage(path) },
    }),
    react(),
  ],
  // The brand's wordmark imports `@/lib/cn`, as it does in the website repo.
  resolve: { alias: [{ find: /^@\//, replacement: `${BRAND}/src/` }] },
  // The 3D views' chunk holds three.js and loads only on a page that shows one.
  build: { chunkSizeWarningLimit: 1200 },
}));
