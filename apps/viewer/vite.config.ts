import { createReadStream, existsSync } from "node:fs";
import { resolve } from "node:path";
import react from "@vitejs/plugin-react";
import { type Connect, defineConfig, type Plugin } from "vite";

const PARTS = resolve(import.meta.dirname, "../../parts");
const GLB_URL = /^\/parts\/([a-z0-9-]+)\/\1\.glb$/;

// Serves each part's GLB from the repo's parts/ at /parts/<id>/<id>.glb. The build leaves
// them out, so the built page holds no model.
const serveGlb: Connect.NextHandleFunction = (request, response, next) => {
  const id = GLB_URL.exec(new URL(request.url ?? "/", "http://localhost").pathname)?.[1];
  const file = id === undefined ? undefined : resolve(PARTS, id, `${id}.glb`);
  if (file === undefined || !existsSync(file)) {
    next();
    return;
  }
  response.setHeader("Content-Type", "model/gltf-binary");
  createReadStream(file).pipe(response);
};

const parts: Plugin = {
  name: "orbe-parts",
  configureServer: (server) => {
    server.middlewares.use(serveGlb);
  },
  configurePreviewServer: (server) => {
    server.middlewares.use(serveGlb);
  },
};

export default defineConfig({
  base: "./",
  plugins: [react(), parts],
  // One bundle, embedded whole in orbe, so splitting it buys nothing.
  build: { chunkSizeWarningLimit: 1500 },
});
