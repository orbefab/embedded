// ORBE_E2E picks the suite. Site runs the same checks twice, once per server.
// A failed check keeps its trace in test-results/.
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { defineConfig, type PlaywrightTestConfig } from "@playwright/test";
import { chromiumPath, LAUNCH_ARGS } from "./e2e/support";

const executablePath = chromiumPath();
const suite = process.env.ORBE_E2E;

const VIEWER = "http://127.0.0.1:4174";
const SITE_VITE_PORT = process.env.ORBE_SITE_VITE_PORT ?? "4173";
const SITE_WRANGLER_PORT = process.env.ORBE_SITE_WRANGLER_PORT ?? "8791";
const SITE_VITE = `http://127.0.0.1:${SITE_VITE_PORT}`;
const SITE_WRANGLER = `http://127.0.0.1:${SITE_WRANGLER_PORT}`;

const viewerServer = {
  command: "pnpm exec vite preview --strictPort --host 127.0.0.1 --port 4174",
  cwd: "apps/viewer",
  url: `${VIEWER}/`,
  reuseExistingServer: false,
  stdout: "ignore" as const,
  timeout: 60_000,
};

const shared: PlaywrightTestConfig = {
  outputDir: "test-results",
  reporter: [
    ["list"],
    ["html", { open: "never", outputFolder: "playwright-report" }],
  ],
  timeout: 180_000,
  workers: 1,
  fullyParallel: false,
  retries: 0,
  use: {
    headless: true,
    trace: "retain-on-failure",
    launchOptions: { executablePath, args: LAUNCH_ARGS },
  },
};

const suites: Record<string, PlaywrightTestConfig> = {
  viewer: {
    ...shared,
    testDir: "apps/viewer/e2e",
    testMatch: "fixture.spec.ts",
    use: { ...shared.use, baseURL: VIEWER, viewport: { width: 1280, height: 720 } },
    webServer: viewerServer,
  },
  block: {
    ...shared,
    testDir: "apps/viewer/e2e",
    testMatch: "block.spec.ts",
    use: { ...shared.use, baseURL: VIEWER, viewport: { width: 1280, height: 720 } },
    webServer: viewerServer,
  },
  headed: {
    ...shared,
    testDir: "apps/viewer/e2e",
    testMatch: "headed.spec.ts",
    use: { ...shared.use, viewport: { width: 1280, height: 720 } },
    webServer: {
      command: "node e2e/headed-server.mjs",
      // The ready line is the signal. The URL is written to .scratch/headed/session.json
      // because the session picks its own port and token.
      wait: { stdout: /ORBE_SHOW_READY=1/ },
      stdout: "pipe",
      gracefulShutdown: { signal: "SIGTERM", timeout: 15_000 },
      timeout: 60_000,
      reuseExistingServer: false,
    },
  },
  world: {
    ...shared,
    testDir: "apps/viewer/e2e",
    testMatch: "world.spec.ts",
    use: { ...shared.use, viewport: { width: 1280, height: 720 } },
    webServer: {
      command: "node e2e/headed-server.mjs",
      env: {
        ...process.env,
        ORBE_SCENE: "packages/session/tests/light-divider/scene.jsonc",
      },
      wait: { stdout: /ORBE_SHOW_READY=1/ },
      stdout: "pipe",
      gracefulShutdown: { signal: "SIGTERM", timeout: 15_000 },
      timeout: 60_000,
      reuseExistingServer: false,
    },
  },
  site: {
    ...shared,
    testDir: "apps/site/e2e",
    testMatch: "site.spec.ts",
    use: { ...shared.use, viewport: { width: 1280, height: 900 } },
    webServer: [
      {
        command: `pnpm exec vite preview --strictPort --host 127.0.0.1 --port ${SITE_VITE_PORT}`,
        cwd: "apps/site",
        url: `${SITE_VITE}/embedded/`,
        name: "vite preview",
        reuseExistingServer: false,
        stdout: "ignore",
        timeout: 120_000,
      },
      {
        command: `pnpm exec wrangler dev --local --ip 127.0.0.1 --port ${SITE_WRANGLER_PORT}`,
        cwd: "apps/site",
        url: `${SITE_WRANGLER}/embedded/`,
        name: "wrangler dev --local",
        reuseExistingServer: false,
        stdout: "ignore",
        timeout: 180_000,
      },
    ],
    projects: [
      { name: "vite preview", use: { baseURL: SITE_VITE } },
      { name: "wrangler dev --local", dependencies: ["vite preview"], use: { baseURL: SITE_WRANGLER } },
    ],
  },
};

if (suite === undefined || !(suite in suites)) {
  console.error("usage: ORBE_E2E=viewer|block|headed|world|site pnpm exec playwright test");
  process.exit(2);
}

if (suite === "viewer") {
  const dist = resolve("apps/viewer/dist");
  if (!existsSync(resolve(dist, "index.html"))) {
    console.error(`No build at ${dist}. Run \`pnpm --filter @orbe/viewer-page build\` first.`);
    process.exit(2);
  }
}

if (suite === "block") {
  for (const [path, how] of [
    [resolve("apps/viewer/dist/index.html"), "pnpm --filter @orbe/viewer-page build"],
    [resolve(".scratch/wasm/pkg/worker.js"), "scripts/wasm.sh .scratch/wasm/pkg"],
    [resolve(".scratch/wasm/registry/index.json"), "orbe registry build . .scratch/wasm/registry"],
  ] as const) {
    if (!existsSync(path)) {
      console.error(`No ${path}. Run \`${how}\` first, or scripts/check-block.sh.`);
      process.exit(2);
    }
  }
}

if (suite === "headed" || suite === "world") {
  const orbe = process.env.ORBE ?? "";
  if (orbe === "" || !existsSync(orbe)) {
    console.error(`No orbe at ${orbe}. Usage: scripts/check-headed.sh [--renders <dir>]`);
    process.exit(2);
  }
}

export default defineConfig(suites[suite]);
