import { mkdirSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { gzipSync } from "node:zlib";
import type { BrowserContext, Page } from "@playwright/test";
import {
  blockKind,
  collectLogs,
  expect,
  ledGlow,
  orbeJson,
  registryDir,
  registryIds,
  rendersDir,
  REPO,
  type RepoBlock,
  shot,
  showBlock,
  test,
  waitGlow,
  waitState,
} from "../../../e2e/support";

const INSTALL = /orbe add|cargo install|orbefab\.com\/orbe\/install\.sh/;
const VIEW = { width: 1280, height: 900 };
const QUERIES = ["led", "5 mm", "kingbright", "board", "resistor 0603", "smd", "avr usb", "nano-board", "Push", "MICROCHIP"];

type Weight = { requests: number; bytes: number; gzip: number; wasm: boolean };

function track(page: Page): { errors: string[]; urls: string[]; weight: () => Promise<Weight> } {
  const errors = collectLogs(page, "errors");
  const urls: string[] = [];
  const fetched: Promise<[string, number, number]>[] = [];
  page.on("request", (request) => urls.push(request.url()));
  page.on("requestfinished", (request) => {
    fetched.push(
      request
        .response()
        .then((response) => response?.body())
        .then((body): [string, number, number] => [
          request.url(),
          body?.length ?? 0,
          body === undefined ? 0 : gzipSync(body, { level: 6 }).length,
        ])
        .catch((): [string, number, number] => [request.url(), 0, 0]),
    );
  });
  const weight = async (): Promise<Weight> => {
    const sizes = await Promise.all(fetched);
    const sum = (at: number) => sizes.reduce((total, size) => total + size[at], 0);
    return { requests: sizes.length, bytes: sum(1), gzip: sum(2), wasm: sizes.some(([url]) => url.endsWith(".wasm")) };
  };
  return { errors, urls, weight };
}

async function instrument(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const workers: { terminated: boolean; created: number }[] = [];
    const created: string[] = [];
    const revoked = new Set<string>();
    const Original = window.Worker;
    window.Worker = class extends Original {
      created: number;
      terminated: boolean;
      constructor(...args: ConstructorParameters<typeof Worker>) {
        super(...args);
        this.created = created.length;
        this.terminated = false;
        workers.push(this);
      }
      override terminate(): void {
        this.terminated = true;
        super.terminate();
      }
    };
    const create = URL.createObjectURL.bind(URL);
    const revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = (blob) => {
      const url = create(blob);
      created.push(url);
      return url;
    };
    URL.revokeObjectURL = (url) => {
      revoked.add(url);
      revoke(url);
    };
    window.orbeTrack = { workers, created, revoked };
  });
}

async function openTracked(browser: { newContext(options: { viewport: { width: number; height: number } }): Promise<BrowserContext> }, url: string) {
  const context = await browser.newContext({ viewport: VIEW });
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const page = await context.newPage();
  const tracked = track(page);
  await instrument(page);
  await page.goto(url);
  return { context, page, ...tracked };
}

const bodyText = (page: Page) => page.evaluate(() => document.body.innerText);

function blockIds(): string[] {
  return registryIds("block");
}

interface RecordFile {
  path: string;
  sha256: string;
}

interface ServedRecord {
  title: string;
  files: RecordFile[];
}

/** The newest record the registry serves for `id`. */
function servedRecord(id: string): ServedRecord {
  const registry = registryDir();
  const index = JSON.parse(readFileSync(join(registry, "index.json"), "utf8")) as {
    items: { name: string; version: string; url: string }[];
  };
  const newest = index.items
    .filter((row) => row.name === id)
    .sort((a, b) => compareVersions(a.version, b.version))
    .at(-1);
  if (newest === undefined) throw new Error(`${id} is not in the registry`);
  return JSON.parse(readFileSync(join(registry, newest.url), "utf8")) as ServedRecord;
}

/** The card line for a part the served Blocks place. A part only in the repo is not on the page. */
async function blocksLine(part: string): Promise<string> {
  const read = await sceneReader();
  const count = blockIds().filter((id) => read.parts(id).some((row) => row.part === part)).length;
  if (count < 1) throw new Error(`${part} is in no Block the registry serves`);
  return count === 1 ? "in 1 Block" : `in ${count} Blocks`;
}

/** Pages of the served Blocks whose scenes place `part`. */
async function blocksPlacing(part: string): Promise<string[]> {
  const read = await sceneReader();
  return blockIds()
    .filter((id) => read.parts(id).some((row) => row.part === part))
    .map((id) => `/orbe/blocks/${id}/`)
    .sort();
}

/** Blocks the registry serves, classified from the scene it serves. */
function releasedBlocks(): RepoBlock[] {
  return blockIds().map((id) => {
    const scene = servedRecord(id).files.find((file) => file.path === "scene.jsonc");
    if (scene === undefined) throw new Error(`${id} has no scene.jsonc in the registry`);
    const text = readFileSync(join(registryDir(), "files", scene.sha256), "utf8");
    return { id, kind: blockKind(text, id) };
  });
}

interface ListedPart {
  ref: string;
  part: string;
  press: boolean;
}

interface SceneReader {
  parts(id: string): ListedPart[];
  board(id: string): string;
  boardTitle(id: string): string;
}

const compareVersions = (a: string, b: string) => {
  const [x, y] = [a.split(".").map(Number), b.split(".").map(Number)];
  for (let i = 0; i < 3; i += 1) if (x[i] !== y[i]) return (x[i] ?? 0) - (y[i] ?? 0);
  return 0;
};

let scenes: Promise<SceneReader> | undefined;

/** The parts each Block places, from the same `sceneParts` the build called. */
function sceneReader(): Promise<SceneReader> {
  scenes ??= openSceneReader();
  return scenes;
}

async function openSceneReader(): Promise<SceneReader> {
  const registry = registryDir();
  const wasmDir = join(REPO, ".scratch/site/wasm");
  const wasm = (await import(pathToFileURL(join(wasmDir, "orbe_wasm.js")).href)) as {
    initSync(options: { module: Uint8Array }): void;
    Block: new () => { add(path: string, bytes: Uint8Array): void };
    sceneParts(block: unknown): string;
  };
  wasm.initSync({ module: readFileSync(join(wasmDir, "orbe_wasm_bg.wasm")) });
  const index = JSON.parse(readFileSync(join(registry, "index.json"), "utf8")) as {
    items: { name: string; version: string; url: string }[];
  };
  const rows = new Map<string, { version: string; url: string }[]>();
  for (const row of index.items) rows.set(row.name, [...(rows.get(row.name) ?? []), row]);
  const records = new Map<string, { type: string; title?: string; board?: string; files: { path: string; sha256: string }[] }>();
  for (const [name, list] of rows) {
    const newest = [...list].sort((a, b) => compareVersions(a.version, b.version)).at(-1);
    if (newest === undefined) continue;
    records.set(name, JSON.parse(readFileSync(join(registry, newest.url), "utf8")));
  }
  const partFiles: [string, Uint8Array][] = [];
  for (const [name, record] of records) {
    if (record.type !== "part") continue;
    const file = record.files.find((entry) => entry.path === "part.jsonc");
    if (file === undefined) continue;
    partFiles.push([`parts/${name}/part.jsonc`, readFileSync(join(registry, "files", file.sha256))]);
  }
  const cache = new Map<string, ListedPart[]>();
  return {
    parts(id) {
      const hit = cache.get(id);
      if (hit !== undefined) return hit;
      const record = records.get(id);
      if (record === undefined || record.type !== "block") throw new Error(`${id} is not a Block in the registry`);
      const block = new wasm.Block();
      for (const [path, bytes] of partFiles) block.add(path, bytes);
      for (const file of record.files) block.add(file.path, readFileSync(join(registry, "files", file.sha256)));
      const listed = JSON.parse(wasm.sceneParts(block)) as ListedPart[];
      cache.set(id, listed);
      return listed;
    },
    board(id) {
      const board = records.get(id)?.board;
      if (board === undefined) throw new Error(`${id} names no board`);
      return board;
    },
    boardTitle(id) {
      const title = records.get(id)?.title;
      if (title === undefined) throw new Error(`${id} has no title in the registry`);
      return title;
    },
  };
}

async function sceneCards(): Promise<Record<string, string>> {
  const read = await sceneReader();
  const cards: Record<string, string> = {};
  for (const id of blockIds()) cards[`/orbe/blocks/${id}/`] = `${read.boardTitle(read.board(id))}, ${read.parts(id).length} parts`;
  return cards;
}

let cachedSummaries: ReadonlyMap<string, string | undefined> | undefined;

/** Each Block's summary, from `orbe blocks --json`, which reads the registry this build wrote. */
function summaries(): ReadonlyMap<string, string | undefined> {
  if (cachedSummaries !== undefined) return cachedSummaries;
  const envelope = orbeJson(process.env.ORBE ?? "", ["blocks"]) as {
    data?: { blocks?: { id: string; description?: string }[] };
  };
  cachedSummaries = new Map((envelope.data?.blocks ?? []).map((row) => [row.id, row.description]));
  return cachedSummaries;
}

/** Sketch, scene and test files the Code view lists, from the record the page serves. */
function codePaths(id: string): string[] {
  return servedRecord(id)
    .files.map((file) => file.path)
    .filter((path) => path === "scene.jsonc" || path.startsWith("tests/") || /^firmware\/.+\.(?:ino|c|cpp|h)$/.test(path))
    .sort();
}

function testCount(id: string): number {
  return servedRecord(id).files.filter((file) => file.path.startsWith("tests/") && file.path.endsWith(".test")).length;
}

const VALUES: Record<string, Record<string, readonly string[]>> = {
  "nano-button-led": { r1: ["220 Ω"], r2: ["10 kΩ"], led1: ["red"], nano: ["button.ino.hex"] },
  "nano-led-fade": { r1: ["220 Ω"], led1: ["red"], nano: ["fade.ino.hex"] },
  "nano-light-comparator": {
    rbot: ["2.2 kΩ"],
    rset: ["4.7 kΩ"],
    rhys: ["68 kΩ"],
    rled: ["220 Ω"],
    led1: ["red"],
    trim: ["10 kΩ", "position 0.5"],
    nano: ["light.ino.hex"],
  },
  "uno-button-led": { r1: ["220 Ω"], r2: ["10 kΩ"], led1: ["red"], uno: ["button.ino.hex"] },
};

async function hydrated(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    for (const node of document.querySelectorAll("body *")) {
      if (Object.keys(node).some((key) => key.startsWith("__reactFiber") || key.startsWith("__reactContainer"))) return true;
    }
    return false;
  });
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve(undefined)))));
}

/** A click that stays on this page must not ask the static host for a server function. */
async function noServerCall(page: Page, act: () => Promise<void>): Promise<void> {
  const title = await page.locator("h1").innerText();
  const urls: string[] = [];
  const errors: string[] = [];
  const onRequest = (request: { url: () => string }) => {
    if (/_serverFn|__tsr\//.test(request.url())) urls.push(request.url());
  };
  page.on("request", onRequest);
  page.on("pageerror", (error) => errors.push(error.message));
  await act();
  page.off("request", onRequest);
  expect(urls, "a navigation called a server function").toEqual([]);
  expect(errors, JSON.stringify(errors)).toEqual([]);
  await expect(page.locator("h1")).toHaveText(title);
  await expect(page.locator("body")).not.toContainText("Something went wrong");
}

async function cardCounts(page: Page): Promise<Record<string, string>> {
  return page.locator('a[href^="/orbe/blocks/"]').evaluateAll((links) =>
    Object.fromEntries(
      links.flatMap((link) => {
        const count = link.querySelector("span.font-mono")?.textContent?.trim();
        return count?.endsWith(" parts") ? [[link.getAttribute("href"), count]] : [];
      }),
    ),
  );
}

function searchNames(query: string): string[] {
  return orbeJson(process.env.ORBE ?? "", ["search", ...query.split(" ")]).data?.parts?.map((part) => part.id) ?? [];
}

const PART_IDS = registryIds("part");
const PART_THIRD = Math.ceil(PART_IDS.length / 3);

// Called from three places in this file. A shard keeps a whole group, and the calls sit
// where each third of a 3-way shard begins, so each job opens about a third of the parts.
function partPages(ids: readonly string[]) {
  const last = ids[ids.length - 1];
  test(`part pages ${ids[0]} through ${last} draw with no viewer error`, async ({ browser, host }) => {
    test.setTimeout(300_000);
    expect(ids.length).toBeGreaterThan(0);
    const context = await browser.newContext();
    const page = await context.newPage();
    const errors = collectLogs(page, "errors");
    for (const id of ids) {
      errors.length = 0;
      const response = await page.goto(`${host.base}/orbe/parts/${id}/`);
      expect(response?.ok(), id).toBe(true);
      const ref = `@${id.replaceAll("-", "_")}`;
      await page.waitForFunction(
        (name) => {
          if (document.querySelector("[data-testid='no-webgl']") !== null) return true;
          if (document.querySelector("[data-testid='errors']") !== null) return true;
          return window.orbeViewer?.get().scene.getObjectByName(name) !== undefined;
        },
        ref,
        { timeout: 60_000 },
      );
      expect(await page.locator('[data-testid="errors"]').count(), id).toBe(0);
      expect(errors, `${id} ${JSON.stringify(errors)}`).toEqual([]);
      if (id === "resistor-axial") {
        await expect(page.getByTestId("shown-at")).toHaveText("Shown at 220 Ω.");
        const sentence = await bodyText(page);
        expect(sentence, id).toContain("above 0 Ω (required)");
        await expect(page.getByRole("heading", { name: "Settings" }), id).toBeVisible();
        expect(sentence, id).not.toContain("0 pΩ");
        expect(sentence, id).not.toContain(", required");
        const bands = await page.evaluate(() => {
          const root = window.orbeViewer.get().scene.getObjectByName("@resistor_axial");
          return ["band1-brown", "band2-black", "band3-orange", "band4-gold"].map((node) => {
            const mesh = root?.getObjectByName(node);
            const material = mesh?.material;
            return material && "color" in material ? `#${material.color.getHexString()}` : "";
          });
        });
        expect(bands, id).toEqual(["#d12b2b", "#d12b2b", "#6b3a1f", "#d4af37"]);
      }
      if (id === "led-5mm") await expect(page.getByTestId("shown-at")).toHaveCount(0);
      if (id === "trimmer-3362p") await expect(page.getByTestId("shown-at")).toHaveCount(0);
    }
    await context.close();
  });
}

// Parallel, so CI can shard groups. A describe that shares one page stays in default mode,
// and that whole describe stays on one shard.
test.describe.configure({ mode: "parallel" });
partPages(PART_IDS.slice(0, PART_THIRD));

test.describe("landing", () => {
  test.describe.configure({ mode: "default" });
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];
  let urls: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors, urls } = await openTracked(browser, `${host.base}/orbe/`));
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the start page says what orbe is", async () => {
    expect(await page.textContent("h1")).toBe("A hardware simulator for the AI agent you already use.");
    expect(await page.title()).toBe("Get orbe | Orbefab");
    const body = await bodyText(page);
    expect(body).toContain("GET ORBE");
    expect(body).toContain("orbe runs real firmware on simulated Arduino boards, so your agent can test a circuit before you build it.");
    expect(body).toContain("Use the AI agent you already have, on your own subscription.");
    expect(body).toContain("Every step is a command an agent can run, and orbe skills get core tells it how.");
    expect(body).toContain("In a terminal for the agent, or live in 3D for you, on every Block page.");
    expect(body).toContain("Boards:");
    expect(body).not.toContain("Boards today:");
  });

  test("the start page has no player and no Block cards", async () => {
    await expect(page.getByTestId("block-stage")).toHaveCount(0);
    await expect(page.locator("canvas")).toHaveCount(0);
    await expect(page.locator("[data-scene-parts]")).toHaveCount(0);
  });

  test("the start page shows no install command", async () => {
    const body = await bodyText(page);
    expect(INSTALL.test(body), body.match(INSTALL)?.[0] ?? "").toBe(false);
    await expect(page.getByTestId("scene-file")).toHaveCount(0);
  });

  test("each step says the CLI is coming", async () => {
    expect(await page.locator('[data-testid="cli-coming"]').count()).toBe(5);
    await shot(page, "site-landing");
  });

  test("the header and footer are the shared ones", async () => {
    const header = page.locator("header");
    await expect(header.getByRole("link", { name: "Blocks" })).toHaveAttribute("href", "/orbe/blocks/");
    await expect(header.getByRole("link", { name: "Parts" })).toHaveAttribute("href", "/orbe/parts/");
    await expect(header.getByRole("link", { name: "Get orbe" })).toHaveAttribute("href", "/orbe/");
    await expect(header.getByRole("link", { name: "Get orbe" })).toHaveAttribute("aria-current", "page");
    await expect(header.locator("a").first()).toHaveAttribute("href", "/");
    expect(await header.innerText()).not.toMatch(/embedded/i);
    const footer = await page.locator("footer").innerText();
    expect(footer.startsWith("© 2026 Orbefab")).toBe(true);
    expect(footer).toContain("Open source under MIT or Apache-2.0.");
    expect(footer).not.toContain("unless a file says otherwise");
    expect(footer).not.toContain("not affiliated");
    await expect(page.locator('footer a[href="https://github.com/orbefab/orbe/releases"]')).toHaveText("Releases");
    await expect(page.locator('footer a[href="https://github.com/orbefab/orbe"]')).toHaveText("GitHub");
    await expect(page.locator('footer a[href="/brand"]')).toHaveText("Brand");
    await expect(page.locator('footer a[href="https://github.com/orbefab/orbe/blob/main/LICENSE-MIT"]')).toHaveText("License");
    expect(await bodyText(page)).not.toContain("Catalog @orbefab");
  });

  test("the landing page logged no error", () => {
    expect(errors, JSON.stringify(errors)).toEqual([]);
  });

  test("the page asks no server for data", ({ host }) => {
    const origin = new URL(host.base).origin;
    const calls = urls.filter((url) => url.startsWith(`${origin}/`) && /_serverFn|__tsr/.test(url));
    expect(calls, JSON.stringify(calls)).toEqual([]);
  });

  test("every file the page loads is under /orbe/", ({ host }) => {
    const origin = new URL(host.base).origin;
    const outside = urls.filter((url) => url.startsWith(`${origin}/`) && !new URL(url).pathname.startsWith("/orbe/"));
    expect(outside, JSON.stringify(outside)).toEqual([]);
  });

  test("the wordmark's font loads from /orbe/fonts/", () => {
    expect(urls.some((url) => url.endsWith("/orbe/fonts/Syne-VariableFont_wght.ttf"))).toBe(true);
  });
});

test.describe("parts search", () => {
  test.describe.configure({ mode: "default" });
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors } = await openTracked(browser, `${host.base}/orbe/parts/`));
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the parts page lists every part without a query", async () => {
    const index = JSON.parse(readFileSync(join(registryDir(), "index.json"), "utf8")) as {
      items: { name: string; type: string; listed?: boolean }[];
    };
    const catalog = new Set(index.items.filter((row) => row.type === "part" && row.listed !== false).map((row) => row.name)).size;
    expect(await page.locator("[data-part]").count()).toBe(catalog);
  });

  test("a card says where the part is used", async () => {
    const line = (id: string) => page.locator(`[data-part="${id}"]`).innerText();
    expect(await page.locator("main")).toContainText("Real parts with their pins, ratings and 3D models.");
    expect(await line("led-5mm")).toContain(await blocksLine("led-5mm"));
    expect(await line("led-5mm")).not.toContain("led-5mm");
    expect(await line("resistor-axial")).toContain(await blocksLine("resistor-axial"));
    expect(await line("lm358-dip8")).toContain(await blocksLine("lm358-dip8"));
    expect(await line("nano-board")).toContain(await blocksLine("nano-board"));
    expect(await line("pin-header-1x15")).toContain("on the Nano (CH340 clone)");
    const unused = await page.locator("[data-part]").evaluateAll((nodes) => nodes.some((node) => (node.textContent ?? "").includes("in no Block yet") || (node.textContent ?? "").includes("Generic")));
    expect(unused).toBe(false);
  });

  for (const query of QUERIES) {
    test(`"${query}" finds what orbe search finds, in its order`, async () => {
      await page.fill('input[name="q"]', query);
      const shown = await page.locator("[data-part]").evaluateAll((items) => items.map((item) => (item as HTMLElement).dataset.part));
      const cli = searchNames(query);
      expect(JSON.stringify(shown), JSON.stringify({ shown, cli })).toBe(JSON.stringify(cli));
    });
  }

  test("the query is in the URL", async () => {
    await page.fill('input[name="q"]', "led");
    expect(await page.evaluate(() => new URL(window.location.href).searchParams.get("q"))).toBe("led");
    await shot(page, "site-parts-search-led");
  });

  test("the parts page logged no error", () => {
    expect(errors, JSON.stringify(errors)).toEqual([]);
  });
});

test.describe("led-5mm", () => {
  test.describe.configure({ mode: "default" });
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];
  let weight: () => Promise<Weight> = async () => ({ requests: 0, bytes: 0, gzip: 0, wasm: false });

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors, weight } = await openTracked(browser, `${host.base}/orbe/parts/led-5mm/`));
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@led_5mm") !== undefined, null, { timeout: 30_000 });
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("led-5mm's page draws its model", async () => {
    expect(await page.evaluate(() => window.orbeViewer?.get().scene.getObjectByName("@led_5mm") !== undefined)).toBe(true);
  });

  test("it lists its three files as downloads", async () => {
    const files = await page
      .locator('[data-testid="license-files"] a[download]')
      .evaluateAll((links) => links.map((link) => [link.textContent, link.getAttribute("href")]));
    const names = files.map(([path]) => path).sort();
    expect(names, JSON.stringify(files)).toEqual(["led-5mm.anchors.json", "led-5mm.glb", "part.jsonc"]);
  });

  test("the license box names the shared licence and lists no per-file column", async () => {
    const box = page.locator('[data-testid="license-files"]');
    await expect(box.locator("summary")).toContainText("MIT OR Apache-2.0");
    await expect(box.locator("th", { hasText: "License" })).toHaveCount(0);
    await expect(box).not.toContainText("Registry record");
  });

  test("a download serves the file's bytes, by the record's SHA-256", async () => {
    const href = await page.locator('[data-testid="license-files"] a[download]', { hasText: /^part\.jsonc$/ }).getAttribute("href");
    const download = await page.evaluate(async (href) => {
      const index = await (await fetch("/orbe/r/index.json")).json();
      const row = index.items.find((item: { name: string }) => item.name === "led-5mm");
      const record = await (await fetch(new URL(row.url, `${location.origin}/orbe/r/index.json`))).json();
      const bytes = new Uint8Array(await (await fetch(href)).arrayBuffer());
      const hex = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map((b) => b.toString(16).padStart(2, "0")).join("");
      return { hex, want: record.files.find((file: { path: string; sha256: string }) => file.path === "part.jsonc").sha256 };
    }, href);
    expect(download.hex, JSON.stringify(download)).toBe(download.want);
  });

  test("it lists DigiKey with its checked date", async () => {
    const body = await bodyText(page);
    expect(/DigiKey\s*2026-11-08/.test(body), body.match(/Where to buy[\s\S]{0,120}/i)?.[0] ?? "").toBe(true);
  });

  test("it shows the ratings as names, with a link to its own datasheet", async () => {
    const section = page.locator("section", { has: page.getByRole("heading", { name: "Ratings" }) });
    await expect(section).toContainText("Current");
    await expect(section).toContainText("30 mA");
    const hrefs = await section.getByRole("link", { name: "Datasheet" }).evaluateAll((links) => links.map((link) => link.getAttribute("href")));
    expect(hrefs.length).toBeGreaterThan(0);
    expect(hrefs.every((href) => href?.endsWith("WP7113ID.pdf")), hrefs.join(" ")).toBe(true);
    const body = await bodyText(page);
    expect(body).not.toContain("Brightness is the average");
    expect(body).not.toContain("Absolute Maximum Ratings");
    expect(body).not.toContain("not affiliated");
    expect(body).not.toContain("No summary yet");
  });

  test("it links the Blocks that place it", async () => {
    const placing = await blocksPlacing("led-5mm");
    const place = await page.locator("dt", { hasText: /^used in$/ }).locator("xpath=following-sibling::dd[1]").innerText();
    expect(place.trim()).toBe(placing.length === 1 ? "1 Block" : `${placing.length} Blocks`);
    const hrefs = await page.locator('[data-testid="used-in"] a').evaluateAll((links) => links.map((link) => link.getAttribute("href")).sort());
    expect(hrefs).toEqual(placing);
  });

  test("it shows what is not simulated", async () => {
    expect(/reverse breakdown/.test(await bodyText(page))).toBe(true);
  });

  test("it shows no install command", async () => {
    const body = await bodyText(page);
    expect(INSTALL.test(body), body.match(INSTALL)?.[0] ?? "").toBe(false);
  });

  test("led-5mm's page logged no error", async () => {
    await page.waitForTimeout(800);
    const loaded = await weight();
    console.log(`     led-5mm: ${loaded.requests} requests, ${loaded.bytes} bytes, ${loaded.gzip} gzipped`);
    await shot(page, "site-part-led-5mm");
    await page.locator('[data-testid="license-files"]').evaluate((box) => {
      box.setAttribute("open", "");
      box.scrollIntoView();
    });
    await shot(page, "site-part-led-5mm-files");
    expect(errors, JSON.stringify(errors)).toEqual([]);
  });
});

test("a released part's ratings link its own datasheet", async ({ page, host }) => {
  const sheets = [
    ["diode-1n4007", "1n4001.pdf"],
    ["npn-2n2222-to92", "p2n2222a-d.pdf"],
    ["nmos-irf520-to220", "irf520.pdf"],
  ] as const;
  for (const [id, file] of sheets) {
    const response = await page.goto(`${host.base}/orbe/parts/${id}/`);
    expect(response?.ok(), id).toBe(true);
    const hrefs = await page
      .locator("section", { has: page.getByRole("heading", { name: "Ratings" }) })
      .getByRole("link", { name: "Datasheet" })
      .evaluateAll((links) => links.map((link) => link.getAttribute("href")));
    expect(hrefs.length, id).toBeGreaterThan(0);
    expect(hrefs.every((href) => href?.endsWith(file)), `${id} ${hrefs.join(" ")}`).toBe(true);
  }
});

partPages(PART_IDS.slice(PART_THIRD, PART_THIRD * 2));

test.describe("breadboard-830", () => {
  test.describe.configure({ mode: "default" });
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors } = await openTracked(browser, `${host.base}/orbe/parts/breadboard-830/`));
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@breadboard_830") !== undefined, null, { timeout: 30_000 });
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("it says holes in a strip are connected and hides the empty pin list", async () => {
    const holes = page.locator("section", { has: page.getByRole("heading", { name: "Holes" }) });
    await expect(holes).toContainText("Holes in a strip are connected.");
    await expect(holes.locator("tbody tr")).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "Pins" })).toHaveCount(0);
    const body = await bodyText(page);
    expect(body).not.toMatch(/No pins yet|No params yet|No ratings yet|Nothing left out yet|No docs yet|No sellers yet|In no Block yet/);
  });

  test("breadboard-830's page logged no error", async () => {
    await page.waitForTimeout(800);
    await shot(page, "site-part-breadboard-830");
    expect(errors, JSON.stringify(errors)).toEqual([]);
  });
});

test.describe("nano-board", () => {
  test.describe.configure({ mode: "default" });
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors } = await openTracked(browser, `${host.base}/orbe/parts/nano-board/`));
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@nano_board.mcu") !== undefined, null, { timeout: 30_000 });
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the Nano's page draws the board with its children", async () => {
    expect(await page.evaluate(() => window.orbeViewer?.get().scene.getObjectByName("@nano_board.usb_serial") !== undefined)).toBe(true);
  });

  test("it is titled as a clone", async () => {
    expect(await page.textContent("h1")).toBe("Nano (CH340 clone)");
  });

  test("its pins are names, with no Reaches column", async () => {
    const pins = page.locator("section", { has: page.getByRole("heading", { name: "Pins" }) });
    await expect(pins).toContainText("D13");
    await expect(pins.getByRole("columnheader", { name: "Reaches" })).toHaveCount(0);
    expect(await bodyText(page)).not.toContain("not affiliated");
  });

  test("it lists the parts it is built from", async () => {
    expect(/Built from[\s\S]*CH340G/i.test(await bodyText(page))).toBe(true);
  });

  test("it lists the Blocks that use it", async () => {
    const body = await bodyText(page);
    expect(/Used in Blocks[\s\S]*Button and LED on a Nano/i.test(body), body.match(/Used in Blocks[\s\S]{0,200}/i)?.[0] ?? body).toBe(true);
    const href = await page.locator('[data-testid="used-in"] a', { hasText: "Button and LED on a Nano" }).getAttribute("href");
    expect(href).toBe("/orbe/blocks/nano-button-led/");
  });

  test("the Nano's page logged no error", async () => {
    await page.waitForTimeout(800);
    await shot(page, "site-part-nano-board");
    expect(errors, JSON.stringify(errors)).toEqual([]);
  });
});

test.describe("uno-board", () => {
  test.describe.configure({ mode: "default" });
  let context: BrowserContext;
  let page: Page;

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page } = await openTracked(browser, `${host.base}/orbe/parts/uno-board/`));
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the Uno is titled as a compatible board", async () => {
    expect(await page.textContent("h1")).toBe("Uno R3 compatible board");
  });

  test("the Uno's files share the box's licence", async () => {
    const box = page.locator('[data-testid="license-files"]');
    await expect(box.locator("summary")).toContainText("MIT OR Apache-2.0");
    await expect(box.locator("th", { hasText: "License" })).toHaveCount(0);
    const names = await box.locator("a[download]").allTextContents();
    expect(
      ["part.jsonc", "uno-board.glb"].every((path) => names.includes(path)) && !names.includes("model.py") && !names.includes("asset.json"),
      JSON.stringify(names),
    ).toBe(true);
  });

  test("the Uno's page links the clone schematic it is drawn from", async () => {
    expect(/RobotDyn UNO R3 CH340G schematic/.test(await bodyText(page))).toBe(true);
  });
});

for (const block of releasedBlocks()) {
  test.describe(`${block.id} on its page`, () => {
    test.describe.configure({ mode: "default" });
    let context: BrowserContext;
    let page: Page;
    let errors: string[] = [];

    test.beforeAll(async ({ browser, host }) => {
      const renders = rendersDir();
      const record = renders !== undefined && block.kind === "light" && host.name === "vite preview" ? renders : undefined;
      if (record !== undefined) mkdirSync(record, { recursive: true });
      context = await browser.newContext({
        viewport: VIEW,
        ...(record !== undefined ? { recordVideo: { dir: record, size: VIEW } } : {}),
      });
      page = await context.newPage();
      errors = collectLogs(page, "errors");
      await page.goto(`${host.base}/orbe/blocks/${block.id}/`);
      await page.waitForFunction(
        (kind) => {
          const scene = window.orbeViewer?.get().scene;
          if (kind === "pixels") return scene?.getObjectByName("@stick.px1")?.getObjectByName("lens") !== undefined;
          const led = scene?.getObjectByName("@led1")?.getObjectByName("body") !== undefined;
          const button = kind !== "button" || scene?.getObjectByName("@btn1")?.getObjectByName("stem") !== undefined;
          return led && button;
        },
        block.kind,
        { timeout: 120_000 },
      );
      await waitState(page, "playing");
    });

    test.afterAll(async () => {
      const video = page?.video() ?? null;
      await context?.close();
      const renders = rendersDir();
      if (video === null || renders === undefined) return;
      const dest = join(renders, `${block.id}-slider.webm`);
      await video.saveAs(dest);
      const raw = await video.path();
      if (raw !== dest) rmSync(raw, { force: true });
    });

    test("opens, plays and shows what it does", async () => {
      await showBlock(page, block, true, `site-${block.id}-playing`);
      expect(await page.locator('[data-testid="errors"]').count()).toBe(0);
    });

    test("shows its summary, and the hold control only when the scene has a press", async () => {
      const caption = summaries().get(block.id);
      if (caption === undefined) await expect(page.getByTestId("block-caption")).toHaveCount(0);
      else await expect(page.getByTestId("block-caption")).toHaveText(caption);
      const presses = (await sceneReader()).parts(block.id).some((part) => part.press);
      if (presses) await expect(page.getByTestId("hold-to-press")).toHaveText("Hold to press");
      else await expect(page.getByTestId("hold-to-press")).toHaveCount(0);
    });

    test("the page logged no error", () => {
      expect(errors, JSON.stringify(errors)).toEqual([]);
    });
  });
}

test.describe("nano-button-led", () => {
  test.describe.configure({ mode: "default" });
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors } = await openTracked(browser, `${host.base}/orbe/blocks/nano-button-led/`));
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 120_000 });
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("Code shows the sketch and the scene, and Copy copies the sketch", async () => {
    await expect(page.locator('[data-testid="view-preview-panel"]')).toBeVisible();
    await expect(page.locator('[data-testid="view-code-panel"]')).toBeHidden();
    await page.locator('[data-testid="view-code"]').click();
    await expect(page.locator('[data-testid="view-code-panel"]')).toBeVisible();
    await expect(page.locator('[data-testid="view-preview-panel"]')).toBeHidden();
    const shown = await page.locator('[data-testid="code-file"]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-path")).sort());
    expect(shown).toEqual(codePaths("nano-button-led"));
    const file = page.locator('[data-testid="code-file"][data-path="firmware/button/button.ino"]');
    const sketch = await file.locator("pre").textContent();
    await file.locator('[data-testid="copy"]').click();
    await expect(file.locator('[data-testid="copy"]')).toHaveText("Copied");
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(sketch);
    await page.locator('[data-testid="view-preview"]').click();
    await expect(page.locator('[data-testid="view-preview-panel"]')).toBeVisible();
  });

  test("the card's part count is the number of parts in the scene file", async () => {
    const refs = (await sceneReader()).parts("nano-button-led").map((part) => part.ref);
    const listed = await page.locator('[data-testid="scene-parts"] li[data-ref]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-ref")));
    expect(listed).toEqual(refs);
    const shown = await page.locator("[data-scene-parts]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-scene-parts")));
    expect(shown.length > 0 && shown.every((count) => count === String(refs.length)), JSON.stringify({ shown, refs })).toBe(true);
  });

  test("it shows the README and no install command", async () => {
    const body = await bodyText(page);
    expect(
      body.includes("A 10k pull-down holds D2 low") && !body.includes("Install it into a project") && !INSTALL.test(body),
      body.match(INSTALL)?.[0] ?? "",
    ).toBe(true);
  });

  test("the Block's files share the box's licence", async () => {
    const box = page.locator('[data-testid="license-files"]');
    await expect(box.locator("summary")).toContainText("(MIT OR Apache-2.0) AND LGPL-2.1-or-later");
    await expect(box.locator("th", { hasText: "License" })).toHaveCount(0);
    const names = await box.locator("a[download]").allTextContents();
    expect(names.includes("firmware/NOTICE") && !names.includes("REUSE.toml"), JSON.stringify(names)).toBe(true);
  });

  test("Space on the hold control presses the button, and the window losing focus releases it", async () => {
    const control = page.getByTestId("hold-to-press");
    await control.focus();
    await page.keyboard.down("Space");
    await waitGlow(page, true);
    expect(await ledGlow(page)).toBeGreaterThan(0.5);
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await waitGlow(page, false);
    expect(await ledGlow(page)).toBeLessThan(0.05);
    await page.keyboard.up("Space");
  });

  test("a pointer on the hold control presses, and leaving the control releases", async () => {
    const control = page.getByTestId("hold-to-press");
    const box = await control.boundingBox();
    if (box === null) throw new Error("the hold control has no box");
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await waitGlow(page, true);
    await page.mouse.move(box.x + box.width / 2, box.y + box.height + 48);
    await waitGlow(page, false);
    await page.mouse.up();
    expect(await ledGlow(page)).toBeLessThan(0.05);
  });

  test("an untrusted script click, with no key or pointer before it, presses briefly", async () => {
    await page.evaluate(() => (document.querySelector("[data-testid='hold-to-press']") as HTMLButtonElement).click());
    await waitGlow(page, true);
    await waitGlow(page, false);
  });

  test("a click still presses after a hold the window losing focus cut short", async () => {
    await page.getByTestId("hold-to-press").focus();
    await page.keyboard.down("Space");
    await waitGlow(page, true);
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await waitGlow(page, false);
    await page.keyboard.up("Space");
    await page.evaluate(() => (document.querySelector("[data-testid='hold-to-press']") as HTMLButtonElement).click());
    await waitGlow(page, true);
    await waitGlow(page, false);
  });

  test("a mouse click on the hold control presses once, not twice", async () => {
    await page.getByTestId("hold-to-press").click();
    // A second, click-driven press would hold for 300 ms.
    await page.waitForTimeout(150);
    expect(await ledGlow(page)).toBeLessThan(0.05);
  });

  test("hiding the page releases a hold", async () => {
    const control = page.getByTestId("hold-to-press");
    await control.focus();
    await page.keyboard.down("Space");
    await waitGlow(page, true);
    await page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await waitGlow(page, false);
    expect(await ledGlow(page)).toBeLessThan(0.05);
    await page.keyboard.up("Space");
  });

  test("the session refused no command", async () => {
    expect(await page.locator('[data-testid="errors"]').count()).toBe(0);
  });

  test("the Block's page logged no error", () => {
    expect(errors, JSON.stringify(errors)).toEqual([]);
  });
});

partPages(PART_IDS.slice(PART_THIRD * 2));

const README = {
  "/orbe/blocks/nano-button-led/": "A 10k pull-down holds D2 low",
  "/orbe/blocks/nano-led-fade/": "fades up and down about every 3 seconds",
  "/orbe/blocks/nano-light-comparator/": "prints dark and a count",
  "/orbe/blocks/uno-button-led/": "A 10k pull-down holds D2 low",
} as const;

test("every Block page shows its README and no install line", async ({ browser, host }) => {
  for (const [path, sentence] of Object.entries(README)) {
    const context = await browser.newContext();
    const page = await context.newPage();
    await page.goto(`${host.base}${path}`);
    const body = await bodyText(page);
    const readme = await page.getByTestId("readme").innerText();
    expect(
      body.includes(sentence) &&
        body.includes("Check it with orbe check scene.jsonc and run its tests with orbe test.") &&
        !body.includes("Install it into a project") &&
        !readme.includes("LGPL AVR") &&
        !readme.includes("firmware/NOTICE") &&
        !readme.includes("490 Hz") &&
        !(path.endsWith("nano-light-comparator/") && (readme.includes("4.76 V") || !readme.includes("2.52 V") || !readme.includes("orbe set light 10"))) &&
        !INSTALL.test(body),
      `${path} ${body.match(INSTALL)?.[0] ?? ""}`,
    ).toBe(true);
    await context.close();
  }
});

test("the Blocks page counts the parts each scene places", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/blocks/`);
  await expect(page.locator("main")).toContainText("Small circuits that work. Play each one live in your browser.");
  expect(await cardCounts(page)).toEqual(await sceneCards());
  await page.goto(`${host.base}/orbe/blocks/nano-led-fade/`);
  const refs = (await sceneReader()).parts("nano-led-fade").map((part) => part.ref);
  const listed = await page.locator('[data-testid="scene-parts"] li[data-ref]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-ref")));
  expect(listed).toEqual(refs);
  await context.close();
});

test("a part that only sits inside a board names that board", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/parts/led-0603/`);
  const value = await page.locator("dt", { hasText: /^on boards$/ }).locator("xpath=following-sibling::dd[1]").innerText();
  expect(value.trim()).toBe("Nano (CH340 clone)");
  expect(await page.locator('[data-testid="used-in"]').count()).toBe(0);
  const href = await page.locator('[data-testid="on-boards"] a').getAttribute("href");
  expect(href).toBe("/orbe/parts/nano-board/");
  await context.close();
});

test("a long hash and the code view stay inside a phone width", async ({ browser, host }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const page = await context.newPage();
  const fits = () => page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1);
  await page.goto(`${host.base}/orbe/parts/led-5mm/`);
  await page.locator('[data-testid="license-files"] summary').click();
  expect(await fits(), "led-5mm").toBe(true);
  await page.goto(`${host.base}/orbe/blocks/nano-button-led/`);
  await page.locator('[data-testid="view-code"]').click();
  expect(await fits(), "nano-button-led code").toBe(true);
  await context.close();
});

test("each Block lists its scene files and its values", async ({ browser, host }) => {
  test.setTimeout(180_000);
  const context = await browser.newContext();
  const page = await context.newPage();
  for (const id of blockIds()) {
    await page.goto(`${host.base}/orbe/blocks/${id}/`);
    await page.locator('[data-testid="view-code"]').click();
    const shown = await page.locator('[data-testid="code-file"]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-path")).sort());
    expect(shown, id).toEqual(codePaths(id));
    const placed = (await sceneReader()).parts(id);
    const refs = placed.map((part) => part.ref);
    const listed = await page.locator('[data-testid="scene-parts"] li[data-ref]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-ref")));
    expect(listed, id).toEqual(refs);
    for (const [ref, texts] of Object.entries(VALUES[id] ?? {})) {
      const row = await page.locator(`[data-testid="scene-parts"] li[data-ref="${ref}"]`).innerText();
      for (const text of texts) expect(row, `${id} ${ref}`).toContain(text);
      expect(row, `${id} ${ref}`).not.toContain(placed.find((part) => part.ref === ref)?.part ?? "missing-type");
    }
    await expect(page.locator("details summary", { hasText: "brings" }), id).toHaveCount(0);
    await expect(page.getByText("See them on the board's page"), id).toHaveCount(0);
    const tests = testCount(id);
    const body = await bodyText(page);
    expect(body, id).toContain(tests === 1 ? "1 test" : `${tests} tests`);
    expect(body, id).not.toContain("parts.lock");
    expect(body, id).toContain("Sketch, scene and tests.");
    const header = page.locator('[data-testid="code-file"][data-path="scene.jsonc"] .border-b');
    await expect(header, id).not.toContainText("MIT");
  }
  await context.close();
});

test("a same-page hash stays on the page, including a second click and a page opened on the hash", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/blocks/nano-button-led/#files`);
  await hydrated(page);
  await expect(page.locator("#files")).toHaveJSProperty("open", true);
  await expect(page.locator('a[href="#files"]')).toHaveCount(0);

  await page.goto(`${host.base}/orbe/parts/led-5mm/`);
  await hydrated(page);
  const used = page.locator('a[href="#used"]');
  await noServerCall(page, async () => {
    await used.click();
    await used.click();
  });
  await expect(page.locator("#used")).toBeVisible();

  await page.goto(`${host.base}/orbe/parts/led-0603/`);
  await hydrated(page);
  const boards = page.locator('a[href="#boards"]');
  await noServerCall(page, async () => {
    await boards.click();
    await boards.click();
  });
  await expect(page.locator("#boards")).toBeVisible();

  await page.goto(`${host.base}/orbe/blocks/nano-button-led/`);
  await hydrated(page);
  const urls: string[] = [];
  const errors: string[] = [];
  page.on("request", (request) => {
    if (/_serverFn|__tsr\//.test(request.url())) urls.push(request.url());
  });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.locator('a[href="/orbe/parts/led-5mm/"]').first().click();
  await page.waitForURL(/\/orbe\/parts\/led-5mm\/$/);
  await expect(page.locator("h1")).toHaveText(servedRecord("led-5mm").title);
  await hydrated(page);
  await page.locator('a[href="/orbe/blocks/nano-button-led/"]').first().click();
  await page.waitForURL(/\/orbe\/blocks\/nano-button-led\/$/);
  await expect(page.locator("h1")).toHaveText("Button and LED on a Nano");
  expect(urls, JSON.stringify(urls)).toEqual([]);
  expect(errors, JSON.stringify(errors)).toEqual([]);
  await expect(page.locator("body")).not.toContainText("Something went wrong");
  await context.close();
});

test("a touch drag on empty stage space does not press the button", async ({ browser, host }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true });
  const page = await context.newPage();
  const errors = collectLogs(page, "errors");
  await page.goto(`${host.base}/orbe/blocks/nano-button-led/`);
  await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 30_000 });
  await waitState(page, "playing");
  await waitGlow(page, false);
  const spot = await page.evaluate(() => {
    const canvas = document.querySelector("canvas");
    if (canvas === null) return null;
    canvas.scrollIntoView({ block: "center" });
    const rect = canvas.getBoundingClientRect();
    const candidates = [
      [rect.left + 24, rect.bottom - 28],
      [rect.right - 28, rect.bottom - 28],
      [rect.left + rect.width * 0.5, rect.top + rect.height * 0.72],
    ];
    for (const [x, y] of candidates) {
      if (document.elementFromPoint(x, y) === canvas) return { x, y };
    }
    return null;
  });
  if (spot === null) throw new Error("no empty canvas point");
  await expect(page.getByText("Loading the Block")).toHaveCount(0);
  const touchAction = await page.evaluate(() => ({
    box: getComputedStyle(document.querySelector('[data-testid="viewer"]')!).touchAction,
    canvas: getComputedStyle(document.querySelector("canvas")!).touchAction,
  }));
  expect(touchAction).toEqual({ box: "pan-y", canvas: "pan-y" });
  const before = await page.evaluate(() => window.orbeViewer.get().camera.position.toArray() as number[]);
  // The hint shows for 1.6 s, and a slow page can take longer than that to reach the check below,
  // so record it when it appears.
  await page.evaluate(() => {
    const seen = window as unknown as { hintSeen?: string };
    new MutationObserver(() => {
      const text = document.querySelector("[data-testid=gesture-hint]")?.textContent;
      if (text) seen.hintSeen = text;
    }).observe(document.body, { childList: true, subtree: true, characterData: true });
  });
  const session = await context.newCDPSession(page);
  await session.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ ...spot, id: 1 }] });
  for (let step = 1; step <= 6; step += 1) {
    await session.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [{ x: spot.x + step * 18, y: spot.y - step * 12, id: 1 }],
    });
  }
  await page.waitForTimeout(200);
  expect(await ledGlow(page)).toBeLessThan(0.05);
  const after = await page.evaluate(() => window.orbeViewer.get().camera.position.toArray() as number[]);
  expect(
    before.every((value, index) => Math.abs(value - (after[index] ?? value)) < 0.5),
    JSON.stringify({ before, after }),
  ).toBe(true);
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { hintSeen?: string }).hintSeen))
    .toBe("Use two fingers to move the view");
  await session.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  const hold = page.getByTestId("hold-to-press");
  await hold.scrollIntoViewIfNeeded();
  const holdBox = await hold.boundingBox();
  if (holdBox === null) throw new Error("the hold control has no box");
  const hx = holdBox.x + holdBox.width / 2;
  const hy = holdBox.y + holdBox.height / 2;
  await session.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: hx, y: hy, id: 2 }] });
  await waitGlow(page, true);
  expect(await ledGlow(page)).toBeGreaterThan(0.5);
  await session.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await waitGlow(page, false);
  expect(errors, JSON.stringify(errors)).toEqual([]);
  await context.close();
});

async function viewDistance(page: Page): Promise<number> {
  return page.evaluate(() => {
    const { camera, controls } = window.orbeViewer.get();
    const position = camera.position.toArray() as number[];
    const target = controls.target.toArray() as number[];
    return Math.hypot(position[0] - target[0], position[1] - target[1], position[2] - target[2]);
  });
}

test("a plain wheel over the stage scrolls the page and Ctrl + wheel zooms", async ({ browser, host }) => {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/blocks/nano-button-led/`);
  await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 60_000 });
  const canvas = page.locator("canvas");
  await canvas.scrollIntoViewIfNeeded();
  const box = await canvas.boundingBox();
  if (box === null) throw new Error("the stage has no box");
  const beforeScroll = await page.evaluate(() => window.scrollY);
  const beforeDistance = await viewDistance(page);
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  // A trusted wheel. page.mouse.wheel does not scroll this page. Reading the hint through a
  // locator would scroll it back into view and undo the wheel.
  const session = await context.newCDPSession(page);
  const ctrl = 2;
  await session.send("Input.dispatchMouseEvent", { type: "mouseWheel", x, y, deltaX: 0, deltaY: 480 });
  await page.waitForFunction((start) => window.scrollY > start + 20, beforeScroll);
  const scrolled = await page.evaluate(() => ({
    scrollY: window.scrollY,
    hint: document.querySelector("[data-testid=gesture-hint]")?.textContent ?? "",
  }));
  expect(scrolled.hint).toBe("Ctrl + scroll to zoom");
  const afterDistance = await viewDistance(page);
  expect(Math.abs(afterDistance - beforeDistance), JSON.stringify({ beforeDistance, afterDistance })).toBeLessThan(0.5);
  const zoomBox = await canvas.boundingBox();
  if (zoomBox === null) throw new Error("the stage has no box");
  const zx = zoomBox.x + zoomBox.width / 2;
  const zy = zoomBox.y + zoomBox.height / 2;
  await session.send("Input.dispatchMouseEvent", { type: "mouseWheel", x: zx, y: zy, deltaX: 0, deltaY: -240, modifiers: ctrl });
  await session.send("Input.dispatchMouseEvent", { type: "mouseWheel", x: zx, y: zy, deltaX: 0, deltaY: -240, modifiers: ctrl });
  await page.waitForTimeout(200);
  const zoomed = await viewDistance(page);
  expect(Math.abs(zoomed - afterDistance), JSON.stringify({ afterDistance, zoomed })).toBeGreaterThan(afterDistance * 0.05);
  await context.close();
});

test("two fingers orbit the stage", async ({ browser, host }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true });
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/blocks/nano-button-led/`);
  await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 60_000 });
  const spot = await page.evaluate(() => {
    const canvas = document.querySelector("canvas");
    if (canvas === null) return null;
    canvas.scrollIntoView({ block: "center" });
    const rect = canvas.getBoundingClientRect();
    return { x: rect.left + rect.width * 0.55, y: rect.top + rect.height * 0.45 };
  });
  if (spot === null) throw new Error("no canvas");
  const before = await page.evaluate(() => window.orbeViewer.get().camera.position.toArray() as number[]);
  const session = await context.newCDPSession(page);
  const finger = (id: number, x: number, y: number) => ({ x, y, id });
  await session.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [finger(1, spot.x, spot.y), finger(2, spot.x + 48, spot.y + 12)],
  });
  for (let step = 1; step <= 8; step += 1) {
    await session.send("Input.dispatchTouchEvent", {
      type: "touchMove",
      touchPoints: [finger(1, spot.x - step * 14, spot.y + step * 6), finger(2, spot.x + 48 - step * 14, spot.y + 12 + step * 6)],
    });
    await page.waitForTimeout(16);
  }
  await session.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await page.waitForTimeout(300);
  const after = await page.evaluate(() => window.orbeViewer.get().camera.position.toArray() as number[]);
  expect(
    before.some((value, index) => Math.abs(value - (after[index] ?? value)) > 0.5),
    JSON.stringify({ before, after }),
  ).toBe(true);
  await context.close();
});

test("the theme button follows a change from another tab, and the next click switches", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/blocks/`);
  const toggle = page.getByRole("button", { name: /Switch to (light|dark) mode/ });
  await expect(toggle).toHaveAttribute("aria-label", "Switch to light mode");
  const other = await context.newPage();
  await other.goto(`${host.base}/orbe/parts/`);
  await other.getByRole("button", { name: "Switch to light mode" }).click();
  await expect(toggle).toHaveAttribute("aria-label", "Switch to dark mode");
  await toggle.click();
  await expect(page.locator("html")).toHaveClass(/\bdark\b/);
  await expect(page.locator("html")).not.toHaveClass(/\blight\b/);
  await expect(toggle).toHaveAttribute("aria-label", "Switch to light mode");
  await context.close();
});

test("hold to press shows the brand outline on keyboard focus", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/blocks/nano-button-led/`);
  await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 60_000 });
  const hold = page.getByTestId("hold-to-press");
  await hold.evaluate((element) => (element as HTMLElement).focus({ focusVisible: true }));
  const outline = await hold.evaluate((element) => {
    const style = getComputedStyle(element);
    return { style: style.outlineStyle, width: style.outlineWidth, color: style.outlineColor };
  });
  expect(outline).toEqual({ style: "solid", width: "2px", color: "rgb(232, 93, 4)" });
  await context.close();
});

test("sitemap.xml lists the site pages and no embed route", async ({ browser, host }) => {
  const blockIds = registryIds("block");
  const partIds = registryIds("part");
  expect(blockIds.length).toBeGreaterThan(0);
  expect(partIds.length).toBeGreaterThan(0);
  const context = await browser.newContext();
  const page = await context.newPage();
  const response = await page.goto(`${host.base}/orbe/sitemap.xml`);
  expect(response?.ok(), String(response?.status())).toBe(true);
  expect(response?.headers()["content-type"] ?? "").toMatch(/xml/);
  const xml = (await response?.text()) ?? "";
  const locs = [...xml.matchAll(/<loc>([^<]*)<\/loc>/g)].map((match) => match[1]);
  expect(locs).toContain("https://orbefab.com/orbe/");
  expect(locs).toContain("https://orbefab.com/orbe/blocks/");
  expect(locs).toContain("https://orbefab.com/orbe/parts/");
  for (const id of blockIds) expect(locs, id).toContain(`https://orbefab.com/orbe/blocks/${id}/`);
  for (const id of partIds) expect(locs, id).toContain(`https://orbefab.com/orbe/parts/${id}/`);
  expect(xml).not.toContain("/embed/");
  expect(locs).toHaveLength(3 + blockIds.length + partIds.length);
  await context.close();
});

test("with no WebGL 2 a part page says so and does not say which value it shows", async ({ browser, host }) => {
  const context = await browser.newContext();
  await context.addInitScript(() => {
    const real = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, kind: string, ...rest: unknown[]) {
      if (kind === "webgl2") return null;
      return (real as (...args: unknown[]) => unknown).call(this, kind, ...rest);
    } as typeof real;
  });
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/parts/resistor-axial/`);
  await expect(page.getByTestId("no-webgl")).toHaveText("Your browser can't show 3D here.");
  await expect(page.getByTestId("shown-at")).toHaveCount(0);
  await context.close();
});

test("an unknown path answers 404 with the site's page and logs no page error", async ({ browser, host }) => {
  for (const path of ["/orbe/no-such-page", "/orbe/parts/no-such-part/"]) {
    const context = await browser.newContext();
    const page = await context.newPage();
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    const response = await page.goto(`${host.base}${path}`);
    await page.waitForTimeout(1000);
    const main = page.locator("main");
    expect(response?.status(), path).toBe(404);
    await expect(page.locator("h1")).toHaveText("Page not found.");
    expect(await page.title(), path).toBe("Page not found | Orbefab");
    await expect(main.getByRole("link", { name: "Get orbe" })).toHaveAttribute("href", "/orbe/");
    await expect(main.getByRole("link", { name: "the Blocks" })).toHaveAttribute("href", "/orbe/blocks/");
    await expect(main.getByRole("link", { name: "the parts" })).toHaveAttribute("href", "/orbe/parts/");
    await expect(main.getByRole("link", { name: "Back home" })).toHaveAttribute("href", "/");
    expect(pageErrors, path).toEqual([]);
    await context.close();
  }
});

function lumaOf(hex: string): number {
  const value = Number.parseInt(hex, 16);
  const red = (value >> 16) & 255;
  const green = (value >> 8) & 255;
  const blue = value & 255;
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

async function stageHex(page: Page): Promise<string> {
  return page.evaluate(() => window.orbeViewer.get().scene.background.getHexString());
}

test("the theme choice persists across a reload and between pages", async ({ browser, host }) => {
  const context = await browser.newContext();
  await context.addInitScript(() => {
    const seen = [];
    localStorage.setItem("theme", "light");
    const watch = (el) => {
      const note = () => seen.push(el.className);
      note();
      new MutationObserver(note).observe(el, { attributes: true, attributeFilter: ["class"] });
    };
    const root = document.documentElement;
    if (root) watch(root);
    else {
      new MutationObserver((_records, observer) => {
        const el = document.documentElement;
        if (!el) return;
        observer.disconnect();
        watch(el);
      }).observe(document, { childList: true });
    }
    Object.assign(window, { themeClasses: seen });
  });
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/parts/`);
  await page.waitForTimeout(300);
  const seen = await page.evaluate(() => (window as unknown as { themeClasses: string[] }).themeClasses);
  expect(seen.every((name) => !name.split(/\s+/).includes("dark")), JSON.stringify(seen)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.classList.contains("light"))).toBe(true);
  await page.reload();
  expect(await page.evaluate(() => localStorage.getItem("theme"))).toBe("light");
  expect(await page.evaluate(() => document.documentElement.classList.contains("light"))).toBe(true);
  await page.goto(`${host.base}/orbe/blocks/`);
  expect(await page.evaluate(() => document.documentElement.classList.contains("light"))).toBe(true);
  expect(await page.evaluate(() => localStorage.getItem("theme"))).toBe("light");
  await context.close();
});

test("the viewer background follows the theme on a Block, a part and the embed", async ({ browser, host }) => {
  test.setTimeout(180_000);
  const context = await browser.newContext({ viewport: VIEW });
  const page = await context.newPage();
  const ready = (name: string) =>
    page.waitForFunction((objectName) => window.orbeViewer?.get().scene.getObjectByName(objectName) !== undefined, name, { timeout: 60_000 });

  await page.goto(`${host.base}/orbe/blocks/nano-button-led/`);
  await ready("@btn1");
  const dark = await stageHex(page);
  expect(dark).toBe("17191c");
  const darkMat = await page.evaluate(() => {
    const mesh = window.orbeViewer.get().scene.getObjectByName("stage-mat");
    const fill = mesh?.material?.uniforms?.fill?.value;
    return typeof fill?.getHexString === "function" ? fill.getHexString() : "";
  });
  expect(darkMat).toBe("202327");
  await page.getByRole("button", { name: "Switch to light mode" }).click();
  await page.waitForFunction(() => window.orbeViewer.get().scene.background.getHexString() === "e9e9ec");
  const light = await stageHex(page);
  expect(light).toBe("e9e9ec");
  expect(lumaOf(light)).toBeGreaterThan(lumaOf(dark));
  const lightControls = await page.getByTestId("controls").evaluate((element) => getComputedStyle(element).color);
  expect(lightControls).not.toBe("");

  await page.goto(`${host.base}/orbe/parts/resistor-axial/`);
  await ready("@resistor_axial");
  expect(await stageHex(page)).toBe("e9e9ec");

  await page.goto(`${host.base}/orbe/blocks/nano-button-led/embed/`);
  await ready("@btn1");
  expect(await stageHex(page)).toBe("e9e9ec");
  await page.evaluate(() => {
    localStorage.setItem("theme", "dark");
    window.dispatchEvent(new StorageEvent("storage", { key: "theme" }));
  });
  await page.waitForFunction(() => window.orbeViewer.get().scene.background.getHexString() === "17191c");
  expect(await stageHex(page)).toBe("17191c");
  await context.close();
});

test("the embed route is the player alone and does not overflow at 320px", async ({ browser, host }) => {
  const context = await browser.newContext({ viewport: { width: 320, height: 640 } });
  const page = await context.newPage();
  const response = await page.goto(`${host.base}/orbe/blocks/nano-button-led/embed/`);
  expect(response?.ok()).toBe(true);
  const headers = response?.headers() ?? {};
  expect((headers["x-frame-options"] ?? "").toLowerCase()).not.toContain("deny");
  expect(headers["content-security-policy"] ?? "").not.toContain("frame-ancestors 'none'");
  expect(await page.locator('meta[name="robots"]').getAttribute("content")).toBe("noindex");
  await expect(page.locator("header")).toHaveCount(0);
  await expect(page.locator("footer")).toHaveCount(0);
  await expect(page.getByTestId("block-embed")).toBeVisible();
  await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 60_000 });
  await expect(page.getByTestId("block-caption")).toBeVisible();
  await expect(page.getByTestId("hold-to-press")).toBeVisible();
  const wide = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth + 1);
  expect(wide).toBe(false);
  await context.close();
});

test("every page title ends with | Orbefab", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  const pages: readonly (readonly [string, string])[] = [
    ["/orbe/", "Get orbe | Orbefab"],
    ["/orbe/blocks/", "Blocks | Orbefab"],
    ["/orbe/parts/", "Parts | Orbefab"],
    ["/orbe/blocks/nano-button-led/", "Button and LED on a Nano | Orbefab"],
    ["/orbe/blocks/nano-button-led/embed/", "Button and LED on a Nano | Orbefab"],
    ["/orbe/parts/resistor-axial/", ""],
    ["/orbe/no-such-page", "Page not found | Orbefab"],
  ];
  for (const [path, exact] of pages) {
    const response = await page.goto(`${host.base}${path}`);
    expect(response !== null, path).toBe(true);
    const title = await page.title();
    expect(title.endsWith(" | Orbefab"), `${path} ${title}`).toBe(true);
    if (exact !== "") expect(title, path).toBe(exact);
    if (path.endsWith("/embed/")) expect(title.startsWith("Button and LED"), title).toBe(true);
  }
  await context.close();
});

test.describe("cache", () => {
  async function head(base: string, path: string, headers: Record<string, string> = {}) {
    const response = await fetch(`${base}${path}`, { headers, redirect: "manual" });
    await response.arrayBuffer();
    return { status: response.status, cache: response.headers.get("cache-control") ?? "", etag: response.headers.get("etag") };
  }

  test("a registry file revalidates against its ETag, and an unchanged one answers 304", async ({ host }) => {
    const index = await (await fetch(`${host.base}/orbe/r/index.json`)).json();
    const row = index.items.find((item: { name: string; url: string }) => item.name === "led-5mm");
    const record = await (await fetch(new URL(row.url, `${host.base}/orbe/r/index.json`))).json();
    const file = `/orbe/r/files/${record.files[0].sha256}`;
    const hit = await head(host.base, file);
    const again = await head(host.base, file, { "If-None-Match": hit.etag ?? "" });
    expect(
      hit.status === 200 && hit.etag !== null && /max-age=31536000, immutable/.test(hit.cache) && again.status === 304,
      JSON.stringify({ hit, again }),
    ).toBe(true);
  });

  test("a missing registry file answers 404 with no immutable cache", async ({ host }) => {
    const miss = await head(host.base, "/orbe/r/files/0000000000000000000000000000000000000000000000000000000000000000");
    expect(miss.status === 404 && !/immutable/.test(miss.cache), JSON.stringify(miss)).toBe(true);
  });

  test("a hashed build file keeps for a year", async ({ host }) => {
    const html = await (await fetch(`${host.base}/orbe/`)).text();
    const asset = html.match(/\/orbe\/assets\/[^"]+\.css/)?.[0];
    const built = await head(host.base, asset ?? "");
    expect(built.status === 200 && /max-age=31536000, immutable/.test(built.cache), JSON.stringify({ asset, built })).toBe(true);
  });

  test("a path with @ answers the record", async ({ host }) => {
    const index = await (await fetch(`${host.base}/orbe/r/index.json`)).json();
    const row = index.items.find((item: { url: string }) => item.url.includes("@"));
    const at = await head(host.base, `/orbe/r/${row.url}`);
    expect(at.status === 200 && /max-age=31536000, immutable/.test(at.cache), JSON.stringify(at)).toBe(true);
  });

  test("a path that only starts with /orbe is not this site's", async ({ host }) => {
    const outside = await head(host.base, "/orbefoo");
    expect(outside.status, JSON.stringify(outside)).toBe(404);
  });
});

test("an old /embedded URL redirects to the same path under /orbe, and the query stays", async ({ host }) => {
  const cases = [
    ["/embedded", "/orbe"],
    ["/embedded/", "/orbe/"],
    ["/embedded/r/led-5mm.json", "/orbe/r/led-5mm.json"],
    ["/embedded/install.sh", "/orbe/install.sh"],
    ["/embedded/parts/?q=led", "/orbe/parts/?q=led"],
  ];
  for (const [from, to] of cases) {
    const response = await fetch(`${host.base}${from}`, { redirect: "manual" });
    await response.arrayBuffer();
    const location = response.headers.get("location") ?? "";
    const got = new URL(location, host.base);
    expect(response.status, `${from} -> ${location}`).toBe(301);
    expect(`${got.pathname}${got.search}`, `${from} -> ${location}`).toBe(to);
  }
  const outside = await fetch(`${host.base}/embeddedfoo`, { redirect: "manual" });
  await outside.arrayBuffer();
  expect(outside.status, "embeddedfoo").toBe(404);
  const direct = await fetch(`${host.base}/orbe/r/index.json`);
  const followed = await fetch(`${host.base}/embedded/r/index.json`);
  expect(followed.status).toBe(200);
  expect(await followed.text()).toBe(await direct.text());
});
