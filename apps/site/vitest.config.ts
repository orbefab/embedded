import { defineConfig } from "vitest/config";

// The site's vite.config.ts starts the Cloudflare plugin and refuses an install flag other than
// "", "0" or "1". These tests only import the catalog readers, so they stay on this config.
export default defineConfig({
  test: { include: ["src/**/*.test.ts"] },
});
