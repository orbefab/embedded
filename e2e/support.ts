import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { test as base, expect, chromium, firefox, webkit, type Page } from "@playwright/test";

export { expect };
// Playwright compiles this file as CommonJS, because the root package has no "type": "module".
export const REPO = resolve(__dirname, "..");
export const HEADED_SESSION = join(REPO, ".scratch/headed/session.json");
export const LAUNCH_ARGS = ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"];

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

export type BlockKind = "button" | "fade" | "light";

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
  return ids.map((id) => ({ id, kind: kindOf(readFileSync(join(root, id, "scene.jsonc"), "utf8"), id) }));
}

function kindOf(scene: string, id: string): BlockKind {
  const code = scene
    .split("\n")
    .map((line) => line.replace(/\/\/.*$/, ""))
    .join("\n");
  if (/"world"\s*:\s*\{[^}]*"light"\s*:/.test(code)) return "light";
  if (/"btn1"\s*:/.test(code)) return "button";
  if (/"led1"\s*:/.test(code)) return "fade";
  throw new Error(`${id} names no button, no fading LED and no light`);
}

export async function simulatedSeconds(page: Page): Promise<number> {
  const text = await textOf(page, "time");
  const time = Number(text?.match(/t = ([0-9.]+) s/)?.[1]);
  if (!Number.isFinite(time)) throw new Error(`no simulated time in ${text}`);
  return time;
}

export function waitSimulatedPast(page: Page, seconds: number): Promise<unknown> {
  return page.waitForFunction(
    (seconds) => {
      const text = document.querySelector('[data-testid="time"]')?.textContent ?? "";
      const time = Number(text.match(/t = ([0-9.]+) s/)?.[1]);
      return Number.isFinite(time) && time > seconds;
    },
    seconds,
    { timeout: 30_000 },
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
    try {
      await waitSimulatedPast(page, start + 1.4);
    } finally {
      await page.evaluate(() => window.clearInterval(window.orbeGlowTimer));
    }
    const span = await page.evaluate(() => window.orbeGlowSpan ?? { min: 0, max: 0 });
    expect(span.max - span.min, `${block.id} brightness`).toBeGreaterThan(0.2);
    expect(await simulatedSeconds(page), `${block.id} time`).toBeGreaterThan(Math.max(1, start + 1.4));
    if (shotName !== null) {
      await page.locator("canvas").scrollIntoViewIfNeeded();
      await shot(page, shotName);
    }
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
