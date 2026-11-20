import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";
import type { BrowserContext, Page } from "@playwright/test";
import { collectLogs, expect, ledGlow, orbeJson, rendersDir, REPO, repoBlocks, shot, showBlock, test, waitGlow, waitState } from "../../../e2e/support";
import { captionOf } from "../src/captions";
import { parseJsonc } from "../src/jsonc";

const INSTALL = /orbe add|cargo install|orbefab\.com\/embedded\/install\.sh/;
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

function licences(page: Page): Promise<Record<string, string | null>> {
  return page
    .locator('[data-testid="license-files"] tbody tr')
    .evaluateAll((rows) => Object.fromEntries(rows.map((row) => [row.cells[0]?.textContent ?? "", row.cells[2]?.textContent ?? null])));
}

function blockIds(): string[] {
  return readdirSync(join(REPO, "blocks"))
    .filter((id) => existsSync(join(REPO, "blocks", id, "scene.jsonc")))
    .sort();
}

function readJsonc(path: string): unknown {
  return parseJsonc(readFileSync(path, "utf8"));
}

function sceneParts(id: string): Record<string, { type?: string }> {
  const scene = readJsonc(join(REPO, "blocks", id, "scene.jsonc")) as { parts?: Record<string, { type?: string }> };
  return scene.parts ?? {};
}

function sceneCards(): Record<string, string> {
  const cards: Record<string, string> = {};
  for (const id of blockIds()) {
    const block = readJsonc(join(REPO, "blocks", id, "block.jsonc")) as { board: string };
    const part = readJsonc(join(REPO, "parts", block.board, "part.jsonc")) as { title: string };
    cards[`/embedded/blocks/${id}/`] = `${part.title}, ${Object.keys(sceneParts(id)).length} parts`;
  }
  return cards;
}

/** Sketch, scene and test files the Code view lists, from the Block's folder. */
function codePaths(id: string): string[] {
  const root = join(REPO, "blocks", id);
  const paths = ["scene.jsonc"];
  const walk = (dir: string) => {
    if (!existsSync(dir)) return;
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) {
        walk(path);
        continue;
      }
      const rel = path.slice(root.length + 1);
      if (rel.startsWith("firmware/") && /\.(?:ino|c|cpp|h)$/.test(rel)) paths.push(rel);
      if (rel.startsWith("tests/")) paths.push(rel);
    }
  };
  walk(join(root, "firmware"));
  walk(join(root, "tests"));
  return paths.sort();
}

function catalogLock(id: string): { own: string[]; lock: number; repeats: [string, number][] } {
  const block = readJsonc(join(REPO, "blocks", id, "block.jsonc")) as { board: string };
  const part = readJsonc(join(REPO, "parts", block.board, "part.jsonc")) as { uses?: Record<string, string> };
  const counts = new Map<string, number>();
  for (const body of Object.values(sceneParts(id))) {
    if (body.type === undefined) continue;
    counts.set(body.type, (counts.get(body.type) ?? 0) + 1);
  }
  const own = Object.keys(part.uses ?? {}).sort();
  return {
    own,
    lock: new Set([...counts.keys(), ...own]).size,
    repeats: [...counts.entries()].filter((entry): entry is [string, number] => entry[1] > 1),
  };
}

const VALUES: Record<string, Record<string, readonly string[]>> = {
  "nano-button-led": { r1: ["220 Ω"], r2: ["10 kΩ"], led1: ["red"], nano: ["button.hex"] },
  "nano-led-fade": { r1: ["220 Ω"], led1: ["red"], nano: ["fade.hex"] },
  "nano-light-comparator": {
    rbot: ["2.2 kΩ"],
    rset: ["4.7 kΩ"],
    rhys: ["68 kΩ"],
    rled: ["220 Ω"],
    led1: ["red"],
    trim: ["10 kΩ", "position 0.5"],
    nano: ["light.hex"],
  },
  "uno-button-led": { r1: ["220 Ω"], r2: ["10 kΩ"], led1: ["red"], uno: ["button.hex"] },
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
  return page.locator('a[href^="/embedded/blocks/"]').evaluateAll((links) =>
    Object.fromEntries(
      links.flatMap((link) => {
        const count = link.querySelector("span.font-mono")?.textContent?.trim();
        return count?.endsWith(" parts") ? [[link.getAttribute("href"), count]] : [];
      }),
    ),
  );
}

function searchNames(query: string): string[] {
  return orbeJson(process.env.ORBE ?? "", ["search", ...query.split(" ")]).data?.parts?.map((part) => part.name) ?? [];
}

/** A top-level scene part whose part file has a press input. A board's reset switch is a child
 * of the board, so it does not count. */
function sceneHasPress(id: string): boolean {
  const scene = parseJsonc(readFileSync(join(REPO, "blocks", id, "scene.jsonc"), "utf8"));
  if (scene === null || typeof scene !== "object" || !("parts" in scene) || scene.parts === null || typeof scene.parts !== "object") {
    throw new Error(`${id} scene has no parts`);
  }
  return Object.values(scene.parts).some((part) => {
    if (part === null || typeof part !== "object" || !("type" in part) || typeof part.type !== "string") return false;
    const file = join(REPO, "parts", part.type, "part.jsonc");
    if (!existsSync(file)) return false;
    const record = parseJsonc(readFileSync(file, "utf8"));
    if (record === null || typeof record !== "object" || !("controls" in record) || record.controls === null || typeof record.controls !== "object") {
      return false;
    }
    return "press" in record.controls;
  });
}

test.describe("landing", () => {
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];
  let urls: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors, urls } = await openTracked(browser, `${host.base}/embedded/`));
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
    expect(body).toContain("Boards today:");
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
    expect(await page.locator('[data-testid="cli-coming"]').count()).toBe(4);
    await shot(page, "site-landing");
  });

  test("the header and footer are the shared ones", async () => {
    const header = page.locator("header");
    await expect(header.getByRole("link", { name: "Blocks" })).toHaveAttribute("href", "/embedded/blocks/");
    await expect(header.getByRole("link", { name: "Parts" })).toHaveAttribute("href", "/embedded/parts/");
    await expect(header.getByRole("link", { name: "Get orbe" })).toHaveAttribute("href", "/embedded/");
    await expect(header.getByRole("link", { name: "Get orbe" })).toHaveAttribute("aria-current", "page");
    await expect(header.locator("a").first()).toHaveAttribute("href", "/");
    expect(await header.innerText()).not.toMatch(/embedded/i);
    const footer = await page.locator("footer").innerText();
    expect(footer.startsWith("© 2026 Orbefab")).toBe(true);
    expect(footer).toContain("orbe and its catalog are open source under MIT or Apache-2.0, unless a file says otherwise. Each file lists its own license.");
    expect(footer).toContain("Maker names say which real part a model matches. Orbefab is not affiliated with or endorsed by those makers, and the models carry no logos.");
    await expect(page.locator('footer a[href="https://github.com/orbefab/embedded/releases"]')).toHaveText("Releases");
    await expect(page.locator('footer a[href="https://github.com/orbefab/embedded"]')).toHaveText("GitHub");
    await expect(page.locator('footer a[href="/brand"]')).toHaveText("Brand");
    await expect(page.locator('footer a[href="https://github.com/orbefab/embedded/blob/main/LICENSE-MIT"]')).toHaveText("License");
    expect(await bodyText(page)).toContain("Catalog @orbefab at /embedded/r/index.json");
  });

  test("the landing page logged no error", () => {
    expect(errors, JSON.stringify(errors)).toEqual([]);
  });

  test("the page asks no server for data", ({ host }) => {
    const origin = new URL(host.base).origin;
    const calls = urls.filter((url) => url.startsWith(`${origin}/`) && /_serverFn|__tsr/.test(url));
    expect(calls, JSON.stringify(calls)).toEqual([]);
  });

  test("every file the page loads is under /embedded/", ({ host }) => {
    const origin = new URL(host.base).origin;
    const outside = urls.filter((url) => url.startsWith(`${origin}/`) && !new URL(url).pathname.startsWith("/embedded/"));
    expect(outside, JSON.stringify(outside)).toEqual([]);
  });

  test("the wordmark's font loads from /embedded/fonts/", () => {
    expect(urls.some((url) => url.endsWith("/embedded/fonts/Syne-VariableFont_wght.ttf"))).toBe(true);
  });
});

test.describe("parts search", () => {
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors } = await openTracked(browser, `${host.base}/embedded/parts/`));
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the parts page lists every part without a query", async () => {
    const parts = join(REPO, "parts");
    const catalog = readdirSync(parts).filter((id) => existsSync(join(parts, id, "part.jsonc"))).length;
    expect(await page.locator("[data-part]").count()).toBe(catalog);
  });

  test("a card says where the part is used", async () => {
    const line = (id: string) => page.locator(`[data-part="${id}"]`).innerText();
    expect(await line("led-5mm")).toContain("in 4 Blocks");
    expect(await line("resistor-axial")).toContain("in 4 Blocks");
    expect(await line("lm358-dip8")).toContain("in 1 Block");
    expect(await line("nano-board")).toContain("in 3 Blocks");
    expect(await line("led-0603")).toContain("on the Nano (CH340 clone)");
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
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];
  let weight: () => Promise<Weight> = async () => ({ requests: 0, bytes: 0, gzip: 0, wasm: false });

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors, weight } = await openTracked(browser, `${host.base}/embedded/parts/led-5mm/`));
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@led_5mm") !== undefined, null, { timeout: 30_000 });
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("led-5mm's page draws its model", async () => {
    expect(await page.evaluate(() => window.orbeViewer?.get().scene.getObjectByName("@led_5mm") !== undefined)).toBe(true);
  });

  test("it says Orbefab is not affiliated with Kingbright", async () => {
    expect(/not affiliated with or endorsed by Kingbright/.test(await bodyText(page))).toBe(true);
  });

  test("it lists all five files as downloads", async () => {
    const files = await page
      .locator('[data-testid="license-files"] a[download]')
      .evaluateAll((links) => links.map((link) => [link.textContent, link.getAttribute("href")]));
    expect(files.length === 5 && files.some(([path]) => path === "led-5mm.glb"), JSON.stringify(files)).toBe(true);
  });

  test("every file shows its licence", async () => {
    const shown = await page.locator('[data-testid="license-files"] td:nth-child(3)').allTextContents();
    expect(shown.length === 5 && shown.every((licence) => licence === "MIT OR Apache-2.0"), JSON.stringify(shown)).toBe(true);
  });

  test("a download serves the file's bytes, by the record's SHA-256", async () => {
    const href = await page.locator('[data-testid="license-files"] a[download]', { hasText: /^part\.jsonc$/ }).getAttribute("href");
    const download = await page.evaluate(async (href) => {
      const record = await (await fetch("/embedded/r/led-5mm@0.1.0.json")).json();
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

  test("it shows the ratings with their sources", async () => {
    const body = await bodyText(page);
    expect(/30 mA/.test(body) && /Absolute Maximum Ratings/.test(body)).toBe(true);
    expect(body).toContain("Brightness is the average current over 20 ms.");
  });

  test("it links the Blocks that place it", async () => {
    const place = await page.locator("dt", { hasText: /^used in$/ }).locator("xpath=following-sibling::dd[1]").innerText();
    expect(place.trim()).toBe("4 Blocks");
    const hrefs = await page.locator('[data-testid="used-in"] a').evaluateAll((links) => links.map((link) => link.getAttribute("href")).sort());
    expect(hrefs).toEqual([
      "/embedded/blocks/nano-button-led/",
      "/embedded/blocks/nano-led-fade/",
      "/embedded/blocks/nano-light-comparator/",
      "/embedded/blocks/uno-button-led/",
    ]);
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

test.describe("breadboard-830", () => {
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors } = await openTracked(browser, `${host.base}/embedded/parts/breadboard-830/`));
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@breadboard_830") !== undefined, null, { timeout: 30_000 });
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("its pins are only its holes", async () => {
    expect(/Pins\s*No pins yet\./i.test(await bodyText(page))).toBe(true);
  });

  test("it lists each strip: two of five holes in each of 63 columns, and four rails", async () => {
    const strips = page.locator("section", { has: page.locator("h2", { hasText: /^Holes$/ }) }).locator("tbody tr");
    expect(await strips.count()).toBe(63 * 2 + 4);
  });

  test("a column's strip and a whole rail are among them", async () => {
    const body = await bodyText(page);
    expect(/a1 to e1\s*a1, b1, c1, d1, e1/.test(body) && /W3 to W7, W9 to W13, .*, W57 to W61\s*W3, W4/.test(body)).toBe(true);
  });

  test("breadboard-830's page logged no error", async () => {
    await page.waitForTimeout(800);
    await shot(page, "site-part-breadboard-830");
    expect(errors, JSON.stringify(errors)).toEqual([]);
  });
});

test.describe("nano-board", () => {
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors } = await openTracked(browser, `${host.base}/embedded/parts/nano-board/`));
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

  test("it says Orbefab is not affiliated with Arduino", async () => {
    expect(/not affiliated with or endorsed by Arduino/.test(await bodyText(page))).toBe(true);
  });

  test("it lists the parts it is built from", async () => {
    expect(/Built from[\s\S]*CH340G/i.test(await bodyText(page))).toBe(true);
  });

  test("it lists the Blocks that use it", async () => {
    const body = await bodyText(page);
    expect(/Used in Blocks[\s\S]*Button and LED on a Nano/i.test(body), body.match(/Used in Blocks[\s\S]{0,200}/i)?.[0] ?? body).toBe(true);
    const href = await page.locator('[data-testid="used-in"] a', { hasText: "Button and LED on a Nano" }).getAttribute("href");
    expect(href).toBe("/embedded/blocks/nano-button-led/");
  });

  test("the Nano's page logged no error", async () => {
    await page.waitForTimeout(800);
    await shot(page, "site-part-nano-board");
    expect(errors, JSON.stringify(errors)).toEqual([]);
  });
});

test.describe("uno-board", () => {
  let context: BrowserContext;
  let page: Page;

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page } = await openTracked(browser, `${host.base}/embedded/parts/uno-board/`));
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the Uno is titled as a compatible board", async () => {
    expect(await page.textContent("h1")).toBe("Uno R3 compatible board");
  });

  test("every file of the Uno, its model.py and part.jsonc too, is MIT OR Apache-2.0", async () => {
    const unoFiles = await licences(page);
    expect(
      ["part.jsonc", "model.py", "uno-board.glb"].every((path) => unoFiles[path] === "MIT OR Apache-2.0"),
      JSON.stringify(unoFiles),
    ).toBe(true);
  });

  test("the Uno's page links the clone schematic it is drawn from", async () => {
    expect(/RobotDyn UNO R3 CH340G schematic/.test(await bodyText(page))).toBe(true);
  });
});

for (const block of repoBlocks()) {
  const caption = captionOf(block.id);
  const presses = sceneHasPress(block.id);
  test.describe(`${block.id} on its page`, () => {
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
      await page.goto(`${host.base}/embedded/blocks/${block.id}/`);
      await page.waitForFunction(
        (kind) => {
          const scene = window.orbeViewer?.get().scene;
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

    test("shows its caption, and the hold control only when the scene has a button", async () => {
      await expect(page.getByTestId("block-caption")).toHaveText(caption);
      if (presses) await expect(page.getByTestId("hold-to-press")).toHaveText("Hold to press");
      else await expect(page.getByTestId("hold-to-press")).toHaveCount(0);
    });

    test("the page logged no error", () => {
      expect(errors, JSON.stringify(errors)).toEqual([]);
    });
  });
}

test.describe("nano-button-led", () => {
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors } = await openTracked(browser, `${host.base}/embedded/blocks/nano-button-led/`));
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
    const refs = Object.keys(sceneParts("nano-button-led"));
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

  test("each Block file shows its own licence", async () => {
    const files = await licences(page);
    expect(
      files["scene.jsonc"] === "MIT OR Apache-2.0" &&
        files["firmware/button/button.ino"] === "MIT OR Apache-2.0" &&
        files["firmware/button.hex"] === "LGPL-2.1-or-later" &&
        files["firmware/LICENSE-runtime"] === "LGPL-2.1-or-later" &&
        files["firmware/LICENSE-libc"] === "BSD-3-Clause",
      JSON.stringify(files),
    ).toBe(true);
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

  test("a click with no key or pointer before it, as a screen reader sends, presses briefly", async () => {
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

const README = {
  "/embedded/blocks/nano-button-led/": "A 10k pull-down holds D2 low",
  "/embedded/blocks/nano-led-fade/": "fades up and down about every 3 seconds",
  "/embedded/blocks/nano-light-comparator/": "prints dark and a count",
  "/embedded/blocks/uno-button-led/": "A 10k pull-down holds D2 low",
} as const;

test("every Block page shows its README and no install line", async ({ browser, host }) => {
  for (const [path, sentence] of Object.entries(README)) {
    const context = await browser.newContext();
    const page = await context.newPage();
    await page.goto(`${host.base}${path}`);
    const body = await bodyText(page);
    expect(
      body.includes(sentence) &&
        body.includes("Check it with orbe check scene.jsonc and run its tests with orbe test.") &&
        !body.includes("Install it into a project") &&
        !INSTALL.test(body),
      `${path} ${body.match(INSTALL)?.[0] ?? ""}`,
    ).toBe(true);
    await context.close();
  }
});

test("the Blocks page counts the parts each scene places", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${host.base}/embedded/blocks/`);
  expect(await cardCounts(page)).toEqual(sceneCards());
  await page.goto(`${host.base}/embedded/blocks/nano-led-fade/`);
  const refs = Object.keys(sceneParts("nano-led-fade"));
  const listed = await page.locator('[data-testid="scene-parts"] li[data-ref]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-ref")));
  expect(listed).toEqual(refs);
  await context.close();
});

test("a part that only sits inside a board names that board", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${host.base}/embedded/parts/led-0603/`);
  const value = await page.locator("dt", { hasText: /^on boards$/ }).locator("xpath=following-sibling::dd[1]").innerText();
  expect(value.trim()).toBe("Nano (CH340 clone)");
  expect(await page.locator('[data-testid="used-in"]').count()).toBe(0);
  const href = await page.locator('[data-testid="on-boards"] a').getAttribute("href");
  expect(href).toBe("/embedded/parts/nano-board/");
  await context.close();
});

test("a long hash and the code view stay inside a phone width", async ({ browser, host }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const page = await context.newPage();
  const fits = () => page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1);
  await page.goto(`${host.base}/embedded/parts/led-5mm/`);
  await page.locator('[data-testid="license-files"] summary').click();
  expect(await fits(), "led-5mm").toBe(true);
  await page.goto(`${host.base}/embedded/blocks/nano-button-led/`);
  await page.locator('[data-testid="view-code"]').click();
  expect(await fits(), "nano-button-led code").toBe(true);
  await context.close();
});

test("each Block lists its scene files, its board's parts, its lock count and its values", async ({ browser, host }) => {
  test.setTimeout(180_000);
  const context = await browser.newContext();
  const page = await context.newPage();
  for (const id of blockIds()) {
    await page.goto(`${host.base}/embedded/blocks/${id}/`);
    await page.locator('[data-testid="view-code"]').click();
    const shown = await page.locator('[data-testid="code-file"]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-path")).sort());
    expect(shown, id).toEqual(codePaths(id));
    const refs = Object.keys(sceneParts(id));
    const listed = await page.locator('[data-testid="scene-parts"] li[data-ref]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-ref")));
    expect(listed, id).toEqual(refs);
    for (const [ref, texts] of Object.entries(VALUES[id] ?? {})) {
      const row = await page.locator(`[data-testid="scene-parts"] li[data-ref="${ref}"]`).innerText();
      for (const text of texts) expect(row, `${id} ${ref}`).toContain(text);
    }
    const { own, lock, repeats } = catalogLock(id);
    const summary = page.locator("details summary", { hasText: "brings" });
    const note = await summary.innerText();
    expect(note, id).toContain(`brings ${own.length}`);
    expect(note, id).toContain(`lists ${lock}`);
    if (repeats.length === 0) expect(note, id).not.toContain("catalog part");
    for (const [partId] of repeats) expect(note, id).toContain(`share one catalog part, ${partId}`);
    const tests = existsSync(join(REPO, "blocks", id, "tests"))
      ? readdirSync(join(REPO, "blocks", id, "tests")).filter((name) => name.endsWith(".test")).length
      : 0;
    expect(await bodyText(page), id).toContain(tests === 1 ? "1 test" : `${tests} tests`);
    await summary.click();
    const hrefs = await page.locator("details ul a").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("href")).sort());
    expect(hrefs, id).toEqual(own.map((partId) => `/embedded/parts/${partId}/`).sort());
  }
  await context.close();
});

test("a same-page hash stays on the page, including a second click and a page opened on the hash", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${host.base}/embedded/blocks/nano-button-led/`);
  await hydrated(page);
  await page.locator('[data-testid="view-code"]').click();
  const caption = page.locator('[data-testid="view-code-panel"] a[href="#files"]');
  await noServerCall(page, async () => {
    await caption.click();
    await caption.click();
  });
  await expect(page.locator("#files")).toHaveJSProperty("open", true);

  const readme = page.locator('[data-testid="readme"] a[href="#files"]');
  await noServerCall(page, async () => {
    await readme.click();
    await readme.click();
  });
  await expect(page.locator("#files")).toHaveJSProperty("open", true);

  await page.goto(`${host.base}/embedded/blocks/nano-button-led/#files`);
  await hydrated(page);
  await expect(page.locator("#files")).toHaveJSProperty("open", true);
  const again = page.locator('[data-testid="readme"] a[href="#files"]');
  await noServerCall(page, () => again.click());
  await expect(page.locator("#files")).toHaveJSProperty("open", true);

  await page.goto(`${host.base}/embedded/parts/led-5mm/`);
  await hydrated(page);
  const used = page.locator('a[href="#used"]');
  await noServerCall(page, async () => {
    await used.click();
    await used.click();
  });
  await expect(page.locator("#used")).toBeVisible();

  await page.goto(`${host.base}/embedded/parts/led-0603/`);
  await hydrated(page);
  const boards = page.locator('a[href="#boards"]');
  await noServerCall(page, async () => {
    await boards.click();
    await boards.click();
  });
  await expect(page.locator("#boards")).toBeVisible();

  await page.goto(`${host.base}/embedded/blocks/nano-button-led/`);
  await hydrated(page);
  const urls: string[] = [];
  const errors: string[] = [];
  page.on("request", (request) => {
    if (/_serverFn|__tsr\//.test(request.url())) urls.push(request.url());
  });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.locator('a[href="/embedded/parts/led-5mm/"]').first().click();
  await page.waitForURL(/\/embedded\/parts\/led-5mm\/$/);
  await expect(page.locator("h1")).toHaveText("5 mm LED");
  await hydrated(page);
  await page.locator('a[href="/embedded/blocks/nano-button-led/"]').first().click();
  await page.waitForURL(/\/embedded\/blocks\/nano-button-led\/$/);
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
  await page.goto(`${host.base}/embedded/blocks/nano-button-led/`);
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
  await expect(page.getByTestId("gesture-hint")).toHaveText("Use two fingers to move the view");
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
  await page.goto(`${host.base}/embedded/blocks/nano-button-led/`);
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
  await page.goto(`${host.base}/embedded/blocks/nano-button-led/`);
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
  await page.goto(`${host.base}/embedded/blocks/`);
  const toggle = page.getByRole("button", { name: /Switch to (light|dark) mode/ });
  await expect(toggle).toHaveAttribute("aria-label", "Switch to light mode");
  const other = await context.newPage();
  await other.goto(`${host.base}/embedded/parts/`);
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
  await page.goto(`${host.base}/embedded/blocks/nano-button-led/`);
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
  const blockIds = readdirSync(join(REPO, "blocks"))
    .filter((id) => existsSync(join(REPO, "blocks", id, "block.jsonc")))
    .sort();
  const partIds = readdirSync(join(REPO, "parts"))
    .filter((id) => existsSync(join(REPO, "parts", id, "part.jsonc")))
    .sort();
  expect(blockIds.length).toBeGreaterThan(0);
  expect(partIds.length).toBeGreaterThan(0);
  const context = await browser.newContext();
  const page = await context.newPage();
  const response = await page.goto(`${host.base}/embedded/sitemap.xml`);
  expect(response?.ok(), String(response?.status())).toBe(true);
  expect(response?.headers()["content-type"] ?? "").toMatch(/xml/);
  const xml = (await response?.text()) ?? "";
  const locs = [...xml.matchAll(/<loc>([^<]*)<\/loc>/g)].map((match) => match[1]);
  expect(locs).toContain("https://orbefab.com/embedded/");
  expect(locs).toContain("https://orbefab.com/embedded/blocks/");
  expect(locs).toContain("https://orbefab.com/embedded/parts/");
  for (const id of blockIds) expect(locs, id).toContain(`https://orbefab.com/embedded/blocks/${id}/`);
  for (const id of partIds) expect(locs, id).toContain(`https://orbefab.com/embedded/parts/${id}/`);
  expect(xml).not.toContain("/embed/");
  expect(locs).toHaveLength(3 + blockIds.length + partIds.length);
  await context.close();
});

test("every part page draws with no viewer error", async ({ browser, host }) => {
  test.setTimeout(600_000);
  const ids = readdirSync(join(REPO, "parts"))
    .filter((id) => existsSync(join(REPO, "parts", id, "part.jsonc")))
    .sort();
  expect(ids.length).toBeGreaterThan(0);
  const context = await browser.newContext();
  const page = await context.newPage();
  const errors = collectLogs(page, "errors");
  for (const id of ids) {
    errors.length = 0;
    const response = await page.goto(`${host.base}/embedded/parts/${id}/`);
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
      expect(sentence, id).toContain("above 0 Ω");
      expect(sentence, id).not.toContain("0 pΩ");
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
    if (id === "led-5mm") await expect(page.getByTestId("shown-at")).toHaveText("Shown at red.");
    if (id === "trimmer-3362p") await expect(page.getByTestId("shown-at")).toHaveCount(0);
  }
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
  await page.goto(`${host.base}/embedded/parts/resistor-axial/`);
  await expect(page.getByTestId("no-webgl")).toBeVisible();
  await expect(page.getByTestId("shown-at")).toHaveCount(0);
  await context.close();
});

test("an unknown path answers 404 with the site's page and logs no page error", async ({ browser, host }) => {
  for (const path of ["/embedded/no-such-page", "/embedded/parts/no-such-part/"]) {
    const context = await browser.newContext();
    const page = await context.newPage();
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    const response = await page.goto(`${host.base}${path}`);
    await page.waitForTimeout(1000);
    expect(
      response?.status() === 404 &&
        (await page.textContent("h1")) === "Nothing moves here." &&
        (await page.title()) === "Not found | Orbefab" &&
        (await bodyText(page)).includes("404 NOT FOUND") &&
        (await bodyText(page)).includes("Try the parts or the Blocks.") &&
        (await page.getByRole("link", { name: "Back home" }).textContent()) === "Back home" &&
        pageErrors.length === 0,
      JSON.stringify({ path, status: response?.status(), title: await page.title(), pageErrors }),
    ).toBe(true);
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
  await page.goto(`${host.base}/embedded/parts/`);
  await page.waitForTimeout(300);
  const seen = await page.evaluate(() => (window as unknown as { themeClasses: string[] }).themeClasses);
  expect(seen.every((name) => !name.split(/\s+/).includes("dark")), JSON.stringify(seen)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.classList.contains("light"))).toBe(true);
  await page.reload();
  expect(await page.evaluate(() => localStorage.getItem("theme"))).toBe("light");
  expect(await page.evaluate(() => document.documentElement.classList.contains("light"))).toBe(true);
  await page.goto(`${host.base}/embedded/blocks/`);
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

  await page.goto(`${host.base}/embedded/blocks/nano-button-led/`);
  await ready("@btn1");
  const dark = await stageHex(page);
  expect(dark).toBe("1b1d21");
  const darkMat = await page.evaluate(() => {
    const mesh = window.orbeViewer.get().scene.getObjectByName("stage-mat");
    const canvas = mesh?.material?.map?.image as HTMLCanvasElement | undefined;
    const pixel = canvas?.getContext("2d")?.getImageData(32, 32, 1, 1).data;
    return pixel === undefined ? "" : `${pixel[0]},${pixel[1]},${pixel[2]}`;
  });
  expect(darkMat).toBe("50,54,60");
  await page.getByRole("button", { name: "Switch to light mode" }).click();
  await page.waitForFunction(() => window.orbeViewer.get().scene.background.getHexString() !== "1b1d21");
  const light = await stageHex(page);
  expect(lumaOf(light)).toBeGreaterThan(lumaOf(dark));
  const lightControls = await page.getByTestId("controls").evaluate((element) => getComputedStyle(element).color);
  expect(lightControls).not.toBe("");

  await page.goto(`${host.base}/embedded/parts/resistor-axial/`);
  await ready("@resistor_axial");
  expect(lumaOf(await stageHex(page))).toBeGreaterThan(lumaOf(dark));

  await page.goto(`${host.base}/embedded/blocks/nano-button-led/embed/`);
  await ready("@btn1");
  expect(lumaOf(await stageHex(page))).toBeGreaterThan(lumaOf(dark));
  await page.evaluate(() => {
    localStorage.setItem("theme", "dark");
    window.dispatchEvent(new StorageEvent("storage", { key: "theme" }));
  });
  await page.waitForFunction(() => window.orbeViewer.get().scene.background.getHexString() === "1b1d21");
  expect(await stageHex(page)).toBe("1b1d21");
  await context.close();
});

test("the embed route is the player alone and does not overflow at 320px", async ({ browser, host }) => {
  const context = await browser.newContext({ viewport: { width: 320, height: 640 } });
  const page = await context.newPage();
  const response = await page.goto(`${host.base}/embedded/blocks/nano-button-led/embed/`);
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
    ["/embedded/", "Get orbe | Orbefab"],
    ["/embedded/blocks/", "Blocks | Orbefab"],
    ["/embedded/parts/", "Parts | Orbefab"],
    ["/embedded/blocks/nano-button-led/", "Button and LED on a Nano | Orbefab"],
    ["/embedded/blocks/nano-button-led/embed/", "Button and LED on a Nano | Orbefab"],
    ["/embedded/parts/resistor-axial/", ""],
    ["/embedded/no-such-page", "Not found | Orbefab"],
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
    const index = await (await fetch(`${host.base}/embedded/r/index.json`)).json();
    const row = index.items.find((item: { name: string; url: string }) => item.name === "led-5mm");
    const record = await (await fetch(new URL(row.url, `${host.base}/embedded/r/index.json`))).json();
    const file = `/embedded/r/files/${record.files[0].sha256}`;
    const hit = await head(host.base, file);
    const again = await head(host.base, file, { "If-None-Match": hit.etag ?? "" });
    expect(hit.status === 200 && hit.etag !== null && again.status === 304, JSON.stringify({ hit, again })).toBe(true);
  });

  test("a missing registry file answers 404 with no immutable cache", async ({ host }) => {
    const miss = await head(host.base, "/embedded/r/files/0000000000000000000000000000000000000000000000000000000000000000");
    expect(miss.status === 404 && !/immutable/.test(miss.cache), JSON.stringify(miss)).toBe(true);
  });

  test("a hashed build file keeps for a year", async ({ host }) => {
    const html = await (await fetch(`${host.base}/embedded/`)).text();
    const asset = html.match(/\/embedded\/assets\/[^"]+\.css/)?.[0];
    const built = await head(host.base, asset ?? "");
    expect(built.status === 200 && /max-age=31536000, immutable/.test(built.cache), JSON.stringify({ asset, built })).toBe(true);
  });

  test("a path with @ answers 307 to the same path with %40", async ({ host }) => {
    const at = await head(host.base, "/embedded/r/led-5mm@0.1.0.json");
    expect(at.status, JSON.stringify(at)).toBe(307);
  });

  test("a path that only starts with /embedded is not this site's", async ({ host }) => {
    const outside = await head(host.base, "/embeddedfoo");
    expect(outside.status, JSON.stringify(outside)).toBe(404);
  });
});
