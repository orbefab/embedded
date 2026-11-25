// ORBE_E2E picks the suite. Site runs the `@vite-smoke` subset on vite preview (or
// install.spec.ts when ORBE_SITE_INSTALL_E2E=1) and the full suite on wrangler dev --local,
// unless ORBE_SITE_SERVERS names one of them.
// A failed check keeps its trace in test-results/.
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { defineConfig, type PlaywrightTestConfig } from "@playwright/test";
import { bundledBrowser, chromiumPath, enginesLaunch, LAUNCH_ARGS } from "./e2e/support";

const suite = process.env.ORBE_E2E;
// The engines suite drives only Firefox and WebKit, so it does not need a Chromium.
const executablePath = suite === "engines" ? undefined : chromiumPath();

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
  knob: {
    ...shared,
    testDir: "apps/viewer/e2e",
    testMatch: "knob.spec.ts",
    use: { ...shared.use, viewport: { width: 1280, height: 720 } },
    webServer: {
      command: "node e2e/headed-server.mjs",
      env: { ...process.env, ORBE_SCENE: "e2e/knob/scene.jsonc" },
      wait: { stdout: /ORBE_SHOW_READY=1/ },
      stdout: "pipe",
      gracefulShutdown: { signal: "SIGTERM", timeout: 15_000 },
      timeout: 60_000,
      reuseExistingServer: false,
    },
  },
  site: siteSuite(),
};

function siteSuite(): PlaywrightTestConfig {
  const which = process.env.ORBE_SITE_SERVERS ?? "both";
  if (which !== "both" && which !== "vite" && which !== "wrangler") {
    console.error(`ORBE_SITE_SERVERS must be both, vite or wrangler, not ${which}`);
    process.exit(2);
  }
  const wantVite = which === "both" || which === "vite";
  const wantWrangler = which === "both" || which === "wrangler";
  const viteServer = {
    command: `pnpm exec vite preview --strictPort --host 127.0.0.1 --port ${SITE_VITE_PORT}`,
    cwd: "apps/site",
    url: `${SITE_VITE}/orbe/`,
    name: "vite preview",
    reuseExistingServer: false,
    stdout: "ignore" as const,
    timeout: 120_000,
  };
  const wranglerServer = {
    command: `pnpm exec wrangler dev --local --ip 127.0.0.1 --port ${SITE_WRANGLER_PORT}`,
    cwd: "apps/site",
    url: `${SITE_WRANGLER}/orbe/`,
    name: "wrangler dev --local",
    reuseExistingServer: false,
    stdout: "ignore" as const,
    timeout: 180_000,
  };
  const webServer = [...(wantVite ? [viteServer] : []), ...(wantWrangler ? [wranglerServer] : [])];
  const projects = [];
  if (wantVite) {
    projects.push({
      name: "vite preview",
      ...(process.env.ORBE_SITE_INSTALL_E2E === "1" ? {} : { grep: /@vite-smoke/ }),
      use: { baseURL: SITE_VITE },
    });
  }
  if (wantWrangler) {
    projects.push({
      name: "wrangler dev --local",
      ...(wantVite ? { dependencies: ["vite preview"] } : {}),
      use: { baseURL: SITE_WRANGLER },
    });
  }
  return {
    ...shared,
    testDir: "apps/site/e2e",
    testMatch: "*.spec.ts",
    use: { ...shared.use, viewport: { width: 1280, height: 900 } },
    webServer: webServer.length === 1 ? webServer[0] : webServer,
    projects,
  };
}

function enginesSuite(): PlaywrightTestConfig {
  const firefoxExecutable = bundledBrowser("Firefox", "firefox");
  const webkitExecutable = bundledBrowser("WebKit", "webkit");
  enginesLaunch("Firefox", firefoxExecutable);
  enginesLaunch("WebKit", webkitExecutable);
  for (const [path, how] of [
    [resolve("apps/viewer/dist/index.html"), "pnpm --filter @orbe/viewer-page build"],
    [resolve("apps/site/dist/client/index.html"), "scripts/site.sh"],
    [resolve(".scratch/wasm/pkg/worker.js"), "scripts/wasm.sh .scratch/wasm/pkg"],
    [resolve(".scratch/wasm/registry/index.json"), "orbe registry build . .scratch/wasm/registry"],
  ] as const) {
    if (!existsSync(path)) {
      console.error(`No ${path}. Run \`${how}\` first.`);
      process.exit(2);
    }
  }
  const viewerPort = process.env.ORBE_VIEWER_PORT ?? "4174";
  const sitePort = process.env.ORBE_SITE_PORT ?? "4175";
  const viewer = `http://127.0.0.1:${viewerPort}`;
  const site = `http://127.0.0.1:${sitePort}`;
  return {
    outputDir: "test-results",
    reporter: shared.reporter,
    timeout: 120_000,
    workers: 1,
    fullyParallel: false,
    retries: 0,
    testDir: "e2e",
    testMatch: "engines.spec.ts",
    use: {
      headless: true,
      trace: "retain-on-failure",
      viewport: { width: 1280, height: 720 },
    },
    projects: [
      { name: "firefox", use: { browserName: "firefox", launchOptions: { executablePath: firefoxExecutable } } },
      { name: "webkit", use: { browserName: "webkit", launchOptions: { executablePath: webkitExecutable } } },
    ],
    webServer: [
      {
        command: `pnpm exec vite preview --strictPort --host 127.0.0.1 --port ${viewerPort}`,
        cwd: "apps/viewer",
        url: `${viewer}/`,
        reuseExistingServer: false,
        stdout: "ignore",
        timeout: 60_000,
      },
      {
        command: `pnpm exec vite preview --strictPort --host 127.0.0.1 --port ${sitePort}`,
        cwd: "apps/site",
        url: `${site}/orbe/`,
        reuseExistingServer: false,
        stdout: "ignore",
        timeout: 120_000,
      },
    ],
  };
}

if (suite === undefined || (suite !== "engines" && !(suite in suites))) {
  console.error("usage: ORBE_E2E=viewer|block|headed|world|knob|site|engines pnpm exec playwright test");
  process.exit(2);
}

if (suite === "viewer" || suite === "block") {
  for (const [path, how] of [
    [resolve("apps/viewer/dist/index.html"), "pnpm --filter @orbe/viewer-page build"],
    [resolve(".scratch/wasm/pkg/worker.js"), "scripts/wasm.sh .scratch/wasm/pkg"],
    [resolve(".scratch/wasm/registry/index.json"), "orbe registry build . .scratch/wasm/registry"],
  ] as const) {
    if (!existsSync(path)) {
      const script = suite === "viewer" ? "scripts/check-viewer-e2e.sh" : "scripts/check-block.sh";
      console.error(`No ${path}. Run \`${how}\` first, or ${script}.`);
      process.exit(2);
    }
  }
}

if (suite === "headed" || suite === "world" || suite === "knob") {
  const orbe = process.env.ORBE ?? "";
  if (orbe === "" || !existsSync(orbe)) {
    console.error(`No orbe at ${orbe}. Usage: scripts/check-headed.sh [--renders <dir>]`);
    process.exit(2);
  }
}

export default defineConfig(suite === "engines" ? enginesSuite() : suites[suite]);
