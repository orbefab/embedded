import { createReadStream, existsSync, statSync } from "node:fs";
import { extname, resolve } from "node:path";
import react from "@vitejs/plugin-react";
import { type Connect, defineConfig, type Plugin } from "vite";

const REPO = resolve(import.meta.dirname, "../..");
const PARTS = resolve(REPO, "parts");
const GLB_URL = /^\/parts\/([a-z0-9-]+)\/\1\.glb$/;
/** Folders served under a URL prefix: the registry and the orbe-wasm package that `?block=`
 * loads, where docs/viewer.md builds them. */
const FOLDERS: readonly (readonly [string, string])[] = [
  ["/registry/", resolve(REPO, ".scratch/wasm/registry")],
  ["/orbe-wasm/", resolve(REPO, ".scratch/wasm/pkg")],
];
const TYPES: Readonly<Record<string, string>> = {
  ".glb": "model/gltf-binary",
  ".js": "text/javascript",
  ".json": "application/json",
  ".wasm": "application/wasm",
};

/** The file a request path names: a part's GLB, or a file in one of FOLDERS. */
function fileAt(path: string): string | undefined {
  const id = GLB_URL.exec(path)?.[1];
  if (id !== undefined) return resolve(PARTS, id, `${id}.glb`);
  for (const [prefix, dir] of FOLDERS) {
    if (!path.startsWith(prefix)) continue;
    const file = resolve(dir, decodeURIComponent(path.slice(prefix.length)));
    return file.startsWith(`${dir}/`) ? file : undefined;
  }
  return undefined;
}

// Serves each part's GLB from the repo's parts/ at /parts/<id>/<id>.glb, and the files of
// FOLDERS. The build leaves them out, so the built page holds no model and no package.
const serve: Connect.NextHandleFunction = (request, response, next) => {
  const file = fileAt(new URL(request.url ?? "/", "http://localhost").pathname);
  if (file === undefined || !existsSync(file) || !statSync(file).isFile()) {
    next();
    return;
  }
  response.setHeader("Content-Type", TYPES[extname(file)] ?? "application/octet-stream");
  createReadStream(file).pipe(response);
};

const parts: Plugin = {
  name: "orbe-parts",
  configureServer: (server) => {
    server.middlewares.use(serve);
  },
  configurePreviewServer: (server) => {
    server.middlewares.use(serve);
  },
};

export default defineConfig({
  base: "./",
  plugins: [react(), parts],
  // One bundle, embedded whole in orbe, so splitting it buys nothing.
  build: { chunkSizeWarningLimit: 1500 },
});
