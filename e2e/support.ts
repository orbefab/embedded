import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { test as base, expect, chromium, type Page } from "@playwright/test";

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
    orbeFixture?: any;
    orbeTrack?: {
      workers: { terminated: boolean; created: number }[];
      created: string[];
      revoked: Set<string>;
    };
    lastPointer?: number;
    realSend?: unknown;
    sets?: unknown[];
    answer?: () => void;
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
  data?: { url?: string; value?: { i: number; on: boolean; [key: string]: unknown }; parts?: { name: string }[] };
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
