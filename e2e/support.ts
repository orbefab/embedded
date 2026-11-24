import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test as base, expect, chromium, firefox, webkit, type Page } from "@playwright/test";

export { expect };
// Playwright compiles this file as CommonJS, because the root package has no "type": "module".
export const REPO = resolve(__dirname, "..");
export const HEADED_SESSION = join(REPO, ".scratch/headed/session.json");
export const LAUNCH_ARGS = ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"];

/** The registry this site build serves. `orbe search` reads the same folder. */
export function registryDir(): string {
  const spec = process.env.ORBE_REGISTRY;
  if (spec !== undefined && spec.startsWith("file://")) {
    const path = fileURLToPath(spec);
    return path.slice(0, path.lastIndexOf("/"));
  }
  return join(REPO, ".scratch/site/registry");
}

const compareVersions = (a: string, b: string) => {
  const [x, y] = [a.split(".").map(Number), b.split(".").map(Number)];
  for (let i = 0; i < 3; i += 1) if (x[i] !== y[i]) return (x[i] ?? 0) - (y[i] ?? 0);
  return 0;
};

/** Part or Block ids the registry serves, one per name. An id that is only in the repo is absent. */
export function registryIds(type: "part" | "block"): string[] {
  const index = join(registryDir(), "index.json");
  const body = JSON.parse(readFileSync(index, "utf8")) as {
    items: { name: string; type: string; version: string }[];
  };
  const best = new Map<string, string>();
  for (const row of body.items) {
    if (row.type !== type) continue;
    const had = best.get(row.name);
    if (had === undefined || compareVersions(had, row.version) < 0) best.set(row.name, row.version);
  }
  return [...best.keys()].sort();
}

const MISSING = [
  "This check installs no browser. Set ORBE_CHROMIUM to a Chromium or Chrome binary,",
  "or install the one Playwright 1.64.1 expects with `pnpm exec playwright install chromium`.",
];

export function chromiumPath(): string {
  let executable = process.env.ORBE_CHROMIUM;
  if (executable === undefined || executable === "") {
    try {
      executable = chromium.executablePath();
    } catch {
      executable = "~/.cache/ms-playwright";
    }
  }
  if (!existsSync(executable)) {
    console.error(`No Chromium at ${executable}.`);
    console.error(MISSING[0]);
    console.error(MISSING[1]);
    process.exit(2);
  }
  return executable;
}

const ENGINES_FIX = [
  "This check installs no browser. Install the Firefox and WebKit Playwright 1.64.1 expects with `pnpm exec playwright install firefox webkit`.",
  "On Ubuntu, a missing system library needs `sudo pnpm exec playwright install-deps firefox webkit`.",
];

function enginesStop(intro: string): never {
  console.error(intro);
  for (const line of ENGINES_FIX) console.error(line);
  process.exit(2);
}

export function bundledBrowser(label: "Firefox" | "WebKit", install: "firefox" | "webkit"): string {
  let executable: string;
  try {
    executable = install === "firefox" ? firefox.executablePath() : webkit.executablePath();
  } catch {
    executable = "~/.cache/ms-playwright";
  }
  if (!existsSync(executable)) enginesStop(`No ${label} at ${executable}.`);
  return executable;
}

// A missing system library makes Playwright exit 1 with its own install-deps line, which names
// neither browser. Launching here turns that into the same exit 2 as a missing Chromium.
const LIBRARY_MISSING = /Host system is missing dependencies|Missing libraries:|cannot open shared object file|error while loading shared libraries|Missing system dependencies/i;

export function enginesLaunch(label: "Firefox" | "WebKit", executablePath: string): void {
  const browser = label === "Firefox" ? "firefox" : "webkit";
  const script = `const { ${browser}: browserType } = require("@playwright/test");
browserType.launch({ executablePath: process.argv[1], headless: true }).then((browser) => browser.close()).catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});`;
  try {
    execFileSync(process.execPath, ["-e", script, executablePath], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 30_000,
      env: { ...process.env, FORCE_COLOR: "0", NO_COLOR: "1" },
    });
  } catch (error) {
    const failed = error as { stdout?: string; stderr?: string; message?: string };
    const text = `${failed.stdout ?? ""}\n${failed.stderr ?? ""}`;
    if (LIBRARY_MISSING.test(text)) enginesStop(`${label} at ${executablePath} cannot start. A system library is missing.`);
    console.error(text.trim() || failed.message);
    process.exit(1);
  }
}

export function rendersDir(): string | undefined {
  const dir = process.env.ORBE_RENDERS;
  return dir === undefined || dir === "" ? undefined : dir;
}

export async function shot(page: Page, name: string): Promise<void> {
  const dir = rendersDir();
  if (dir === undefined) return;
  mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: join(dir, `${name}.png`) });
}

type Host = { base: string; name: string };

export const test = base.extend<{}, { host: Host }>({
  host: [
    async ({}, use, workerInfo) => {
      await use({
        base: String(workerInfo.project.use.baseURL ?? ""),
        name: workerInfo.project.name,
      });
    },
    { scope: "worker" },
  ],
});

declare global {
  interface Window {
    orbeViewer?: { get(): any };
    orbeTrack?: {
      workers: { terminated: boolean; created: number }[];
      created: string[];
      revoked: Set<string>;
    };
    orbeGlowSpan?: { min: number; max: number };
    orbeGlowTimer?: number;
    lastPointer?: number;
  }
}

export function collectLogs(page: Page, mode: "all" | "errors"): string[] {
  const logs: string[] = [];
  if (mode === "errors") {
    page.on("pageerror", (error) => logs.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") logs.push(message.text());
    });
    return logs;
  }
  page.on("console", (message) => logs.push(message.text()));
  page.on("pageerror", (error) => logs.push(`pageerror: ${error.message}`));
  return logs;
}

export function pointOn(page: Page, ref: string, node?: string, canvas = false): Promise<{ x: number; y: number }> {
  return page.evaluate(
    ([ref, node, canvas]) => {
      const { scene, camera, size } = window.orbeViewer.get();
      const group = scene.getObjectByName(ref);
      // An omitted node is the group itself. Callers pass "" for that.
      const object = node === "" ? group : group?.getObjectByName(node);
      if (object === undefined) throw new Error(`no ${ref} ${node}`);
      let box;
      object.traverse((mesh) => {
        if (!mesh.isMesh) return;
        mesh.geometry.computeBoundingBox();
        const own = mesh.geometry.boundingBox.clone().applyMatrix4(mesh.matrixWorld);
        box = box === undefined ? own : box.union(own);
      });
      const at = box.getCenter(box.min.clone()).project(camera);
      const origin = canvas ? document.querySelector("canvas").getBoundingClientRect() : { left: 0, top: 0 };
      return { x: origin.left + ((at.x + 1) / 2) * size.width, y: origin.top + ((1 - at.y) / 2) * size.height };
    },
    [ref, node ?? "", canvas] as const,
  );
}

export function textOf(page: Page, id: string): Promise<string | null> {
  return page.evaluate((id) => document.querySelector(`[data-testid="${id}"]`)?.textContent ?? null, id);
}

export function waitState(page: Page, state: string): Promise<unknown> {
  return page.waitForFunction(
    (state) => document.querySelector('[data-testid="state"]')?.textContent === state,
    state,
    { timeout: 10_000 },
  );
}

export function ledGlow(page: Page, ref = "@led1"): Promise<number> {
  return page.evaluate(
    (ref) => window.orbeViewer.get().scene.getObjectByName(ref).getObjectByName("body").material.emissiveIntensity,
    ref,
  );
}

export function waitGlow(page: Page, above: boolean, ref = "@led1"): Promise<unknown> {
  return page.waitForFunction(
    ({ above, ref }) => {
      const glow = window.orbeViewer.get().scene.getObjectByName(ref).getObjectByName("body").material.emissiveIntensity;
      return above ? glow > 0.5 : glow < 0.05;
    },
    { above, ref },
    { timeout: 10_000 },
  );
}

export interface OrbeEnvelope {
  ok: boolean;
  state?: string;
  t_ps?: number;
  error?: { code?: string; message?: string; hint?: string };
  data?: { url?: string; value?: { i: number; on: boolean; [key: string]: unknown }; parts?: { id: string }[] };
}

export function orbeJson(binary: string, args: string[], env: NodeJS.ProcessEnv = process.env): OrbeEnvelope {
  try {
    return JSON.parse(execFileSync(binary, [...args, "--json"], { cwd: REPO, env, encoding: "utf8" }));
  } catch (error) {
    // A refused command exits non-zero and still prints the envelope.
    const stdout = (error as { stdout?: unknown }).stdout;
    if (typeof stdout === "string" && stdout.startsWith("{")) return JSON.parse(stdout);
    throw error;
  }
}

export function headedSession(): { ok: boolean; url: string | null; error: unknown; state?: string; runtime: string } {
  return JSON.parse(readFileSync(HEADED_SESSION, "utf8"));
}

export type BlockKind = "button" | "digit" | "fade" | "light" | "pixels" | "rgb" | "screen" | "shift" | "tone";

export interface RepoBlock {
  id: string;
  kind: BlockKind;
}

/** Blocks under `blocks/`. A scene this cannot classify fails the check. */
export function repoBlocks(): RepoBlock[] {
  const root = join(REPO, "blocks");
  const ids = readdirSync(root)
    .filter((id) => existsSync(join(root, id, "block.jsonc")))
    .sort();
  if (ids.length === 0) throw new Error("blocks/ has no Block");
  return ids.map((id) => ({ id, kind: blockKind(readFileSync(join(root, id, "scene.jsonc"), "utf8"), id) }));
}

export function blockKind(scene: string, id: string): BlockKind {
  const code = scene
    .split("\n")
    .map((line) => line.replace(/\/\/.*$/, ""))
    .join("\n");
  if (/"type"\s*:\s*"display-7seg-cc"/.test(code) || /"type"\s*:\s*"display-7seg-ca"/.test(code)) return "digit";
  if (/"type"\s*:\s*"neopixel-stick-8"/.test(code)) return "pixels";
  if (/"type"\s*:\s*"oled-ssd1306-128x64"/.test(code)) return "screen";
  if (/"type"\s*:\s*"hc595-dip16"/.test(code)) return "shift";
  if (/"type"\s*:\s*"esp32s3-devkitc-1"/.test(code)) return "rgb";
  if (/"world"\s*:\s*\{[^}]*"light"\s*:/.test(code)) return "light";
  if (/"btn1"\s*:/.test(code)) return "button";
  if (/"type"\s*:\s*"buzzer-12mm"/.test(code)) return "tone";
  if (/"led1"\s*:/.test(code)) return "fade";
  throw new Error(
    `${id} names no button, no fading LED, no light, no pixel stick, no screen, no 7-segment display, no 74HC595, no buzzer and no DevKitC-1`,
  );
}

export async function simulatedSeconds(page: Page): Promise<number> {
  const text = await textOf(page, "time");
  const time = Number(text?.match(/t = ([0-9.]+) s/)?.[1]);
  if (!Number.isFinite(time)) throw new Error(`no simulated time in ${text}`);
  return time;
}

export function waitSimulatedPast(page: Page, seconds: number, timeout = 30_000): Promise<unknown> {
  return page.waitForFunction(
    (seconds) => {
      const text = document.querySelector('[data-testid="time"]')?.textContent ?? "";
      const time = Number(text.match(/t = ([0-9.]+) s/)?.[1]);
      return Number.isFinite(time) && time > seconds;
    },
    seconds,
    { timeout },
  );
}

const LIGHT = '[data-testid="world-light"]';
const LIGHT_VALUE = '[data-testid="world-light-value"]';

// Six stops separate 300 lx from 10 lx on the log light slider.
const LIGHT_STEPS = 6;

async function stepLight(page: Page, key: "ArrowLeft" | "ArrowRight", want: string): Promise<void> {
  const thumb = page.locator(LIGHT);
  await thumb.scrollIntoViewIfNeeded();
  await thumb.focus();
  for (let k = 0; k < LIGHT_STEPS; k += 1) await page.keyboard.press(key);
  await page.waitForFunction(
    ([field, want]) => document.querySelector(field)?.getAttribute("data-frame") === want,
    [LIGHT_VALUE, want] as const,
    { timeout: 10_000 },
  );
}

const FADE_CYCLE_S = 3.1;

// The OLED Block's sketch turns the display on and starts sending its frame at about 130 ms
// of simulated time, and the frame is whole by 170 ms. Its I2C traffic runs at a few ms a
// second in the Worker, so the check waits on the glass, not on the clock, and asks only for
// the first lit pixels the session sends.
const SCREEN_WAIT_MS = 150_000;

/** Waits until the page draws a frame the OLED holds: the `glass` node of `@oled.display`
 * shows, as its emissive map, at least one white pixel. */
async function showScreen(page: Page, shotName: string | null): Promise<void> {
  await page.waitForFunction(
    () => {
      const glass = window.orbeViewer.get().scene.getObjectByName("@oled.display")?.getObjectByName("glass");
      const material = glass?.material ?? glass?.children?.[0]?.material;
      const data = material?.emissiveMap?.image?.data;
      if (data === undefined) return false;
      for (let k = 0; k < data.length; k += 4) if (data[k] === 255) return true;
      return false;
    },
    undefined,
    { timeout: SCREEN_WAIT_MS },
  );
  if (shotName !== null) {
    await page.locator("canvas").scrollIntoViewIfNeeded();
    await shot(page, shotName);
  }
}

/** Eight LEDs on a 74HC595. QA is led1, the least significant bit of 0x55. */
async function showShift(page: Page, id: string, shotName: string | null): Promise<void> {
  await waitSimulatedPast(page, 0.2);
  const glows = await page.evaluate(() =>
    [1, 2, 3, 4, 5, 6, 7, 8].map((n) => {
      const body = window.orbeViewer.get().scene.getObjectByName(`@led${n}`)?.getObjectByName("body");
      const material = body?.material;
      return typeof material?.emissiveIntensity === "number" ? material.emissiveIntensity : -1;
    }),
  );
  glows.forEach((glow, index) => {
    if (index % 2 === 0) expect(glow, `${id} led${index + 1}`).toBeGreaterThan(0.15);
    else expect(glow, `${id} led${index + 1}`).toBeLessThan(0.05);
  });
  if (shotName !== null) {
    await page.locator("canvas").scrollIntoViewIfNeeded();
    await shot(page, shotName);
  }
}

function lensColour(page: Page): Promise<{ r: number; g: number } | null> {
  return page.evaluate(() => {
    const node = window.orbeViewer.get().scene.getObjectByName("@board.led")?.getObjectByName("lens");
    const material = node?.material ?? node?.children?.[0]?.material;
    if (material === undefined) return null;
    return { r: material.emissive.r, g: material.emissive.g };
  });
}

/** The DevKitC-1 Block: a press changes `@board.led` from red to green. */
async function showRgb(page: Page, block: RepoBlock, canvas: boolean, shotName: string | null): Promise<void> {
  await page.waitForFunction(() => {
    const node = window.orbeViewer.get().scene.getObjectByName("@board.led")?.getObjectByName("lens");
    const material = node?.material ?? node?.children?.[0]?.material;
    return material !== undefined && material.emissive.r > material.emissive.g && material.emissiveIntensity > 0.05;
  }, null, { timeout: 120_000 });
  const wall0 = Date.now();
  const sim0 = await simulatedSeconds(page);
  const stem = await pointOn(page, "@btn1", "stem", canvas);
  await page.mouse.move(stem.x, stem.y);
  await page.mouse.down();
  await page.waitForFunction(() => {
    const node = window.orbeViewer.get().scene.getObjectByName("@board.led")?.getObjectByName("lens");
    const material = node?.material ?? node?.children?.[0]?.material;
    return material !== undefined && material.emissive.g > material.emissive.r;
  }, null, { timeout: 120_000 });
  const rate = ((await simulatedSeconds(page)) - sim0) / ((Date.now() - wall0) / 1000);
  console.log(`${block.id}: ${rate.toFixed(3)} simulated seconds per wall second`);
  expect(rate, `${block.id} time`).toBeGreaterThan(0);
  expect((await lensColour(page))?.g).toBeGreaterThan((await lensColour(page))?.r ?? 1);
  if (shotName !== null) {
    await page.locator("canvas").scrollIntoViewIfNeeded();
    await shot(page, shotName);
  }
  await page.mouse.up();
}

/** The 7-segment Block: after a short count, at least one segment is lit. */
async function showDigit(page: Page, block: RepoBlock, shotName: string | null): Promise<void> {
  await waitSimulatedPast(page, 0.3);
  const brightest = await page.evaluate(() => {
    const root = window.orbeViewer.get().scene.getObjectByName("@disp");
    return ["a", "b", "c", "d", "e", "f", "g"].reduce((best, name) => {
      const node = root?.getObjectByName(`seg-${name}`);
      const material = node?.material ?? node?.children?.[0]?.material;
      return Math.max(best, Number(material?.emissiveIntensity ?? 0));
    }, 0);
  });
  expect(brightest, `${block.id} segment`).toBeGreaterThan(0.2);
  expect(await simulatedSeconds(page), `${block.id} time`).toBeGreaterThan(0.3);
  if (shotName !== null) {
    await page.locator("canvas").scrollIntoViewIfNeeded();
    await shot(page, shotName);
  }
}

/** Play, require simulated time to move, and check the behaviour the scene declares. */
export async function showBlock(page: Page, block: RepoBlock, canvas: boolean, shotName: string | null): Promise<void> {
  await page.locator("canvas").scrollIntoViewIfNeeded();
  if ((await textOf(page, "state")) !== "playing") {
    await page.click('[data-testid="play"]');
    await waitState(page, "playing");
  }
  // The site is already playing when its page opens, so measure from here, not from t = 0.
  const start = await simulatedSeconds(page);
  if (block.kind === "fade") {
    await page.evaluate(() => {
      window.orbeGlowSpan = undefined;
      const sample = () => {
        const glow = Number(window.orbeViewer.get().scene.getObjectByName("@led1").getObjectByName("body").material.emissiveIntensity);
        const span = window.orbeGlowSpan ?? { min: glow, max: glow };
        span.min = Math.min(span.min, glow);
        span.max = Math.max(span.max, glow);
        window.orbeGlowSpan = span;
      };
      sample();
      window.orbeGlowTimer = window.setInterval(sample, 50);
    });
    // The fade sketch climbs and falls in 102 steps of 30 ms, so watch one whole cycle.
    try {
      await waitSimulatedPast(page, start + FADE_CYCLE_S);
    } finally {
      await page.evaluate(() => window.clearInterval(window.orbeGlowTimer));
    }
    const span = await page.evaluate(() => window.orbeGlowSpan ?? { min: 0, max: 0 });
    expect(span.max - span.min, `${block.id} brightness`).toBeGreaterThan(0.2);
    expect(await simulatedSeconds(page), `${block.id} time`).toBeGreaterThan(Math.max(1, start + FADE_CYCLE_S));
    if (shotName !== null) {
      await page.locator("canvas").scrollIntoViewIfNeeded();
      await shot(page, shotName);
    }
    return;
  }
  if (block.kind === "shift") {
    await showShift(page, block.id, shotName);
    return;
  }
  if (block.kind === "rgb") {
    await showRgb(page, block, canvas, shotName);
    return;
  }
  if (block.kind === "tone") {
    // A transistor switching the coil takes a few hundred steps on each edge.
    await waitSimulatedPast(page, Math.max(1, start), 60_000);
    expect(await simulatedSeconds(page), `${block.id} time`).toBeGreaterThan(Math.max(1, start));
    if (shotName !== null) {
      await page.locator("canvas").scrollIntoViewIfNeeded();
      await shot(page, shotName);
    }
    return;
  }
  if (block.kind === "digit") {
    await showDigit(page, block, shotName);
    return;
  }
  if (block.kind === "pixels") {
    await waitSimulatedPast(page, Math.max(0.05, start));
    const colours = await page.evaluate(() => {
      const scene = window.orbeViewer.get().scene;
      return [1, 2, 3, 4, 5, 6, 7, 8].map((n) => {
        const node = scene.getObjectByName(`@stick.px${n}`)?.getObjectByName("lens");
        const material = node?.material ?? node?.children?.[0]?.material;
        if (material === undefined) return null;
        return { r: material.emissive.r, g: material.emissive.g, b: material.emissive.b, i: material.emissiveIntensity };
      });
    });
    expect(colours.every((colour) => colour !== null && colour.i > 0.2), `${block.id} glow`).toBe(true);
    const seen = new Set(colours.map((colour) => `${colour!.r.toFixed(3)},${colour!.g.toFixed(3)},${colour!.b.toFixed(3)}`));
    expect(seen.size, `${block.id} colours`).toBeGreaterThan(1);
    expect(await simulatedSeconds(page), `${block.id} time`).toBeGreaterThan(0.05);
    if (shotName !== null) {
      await page.locator("canvas").scrollIntoViewIfNeeded();
      await shot(page, shotName);
    }
    return;
  }
  if (block.kind === "screen") {
    await showScreen(page, shotName);
    return;
  }
  await waitSimulatedPast(page, Math.max(1, start));
  expect(await simulatedSeconds(page), `${block.id} time`).toBeGreaterThan(1);
  if (block.kind === "button") {
    const stem = await pointOn(page, "@btn1", "stem", canvas);
    await page.mouse.move(stem.x, stem.y);
    await page.mouse.down();
    await waitGlow(page, true);
    expect(await ledGlow(page)).toBeGreaterThan(0.5);
    if (shotName !== null) {
      await page.locator("canvas").scrollIntoViewIfNeeded();
      await shot(page, shotName);
    }
    await page.mouse.up();
    await waitGlow(page, false);
    expect(await ledGlow(page)).toBeLessThan(0.05);
    return;
  }
  await page.waitForSelector(LIGHT, { timeout: 30_000 });
  const frame = await page.locator(LIGHT_VALUE).getAttribute("data-frame");
  expect(frame, `${block.id} starts in the light`).toBe("300");
  await waitGlow(page, false);
  await stepLight(page, "ArrowLeft", "10");
  await waitGlow(page, true);
  expect(await ledGlow(page)).toBeGreaterThan(0.5);
  if (shotName !== null) {
    await page.locator("canvas").scrollIntoViewIfNeeded();
    await shot(page, shotName);
  }
  await stepLight(page, "ArrowRight", "300");
  await waitGlow(page, false);
  expect(await ledGlow(page)).toBeLessThan(0.05);
}
