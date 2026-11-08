import { resolve } from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const BRAND = resolve(import.meta.dirname, "../../packages/brand/website");

// build.mjs sets the inputs and output folders. The brand copy's public folder holds the
// favicons, the logo and the wordmark font, served beside the pages.
export default defineConfig({
  base: "/embedded/",
  publicDir: resolve(BRAND, "public"),
  plugins: [react(), tailwindcss()],
  // The brand's wordmark imports `@/lib/cn`, as it does in the website repo.
  resolve: { alias: [{ find: /^@\//, replacement: `${BRAND}/src/` }] },
  // The 3D views' chunk holds three.js and loads only on a page that shows one.
  build: { chunkSizeWarningLimit: 1200 },
});
