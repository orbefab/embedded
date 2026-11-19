import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";
import type { BrowserContext, Page } from "@playwright/test";
import { collectLogs, expect, ledGlow, orbeJson, rendersDir, REPO, repoBlocks, shot, showBlock, test, waitGlow, waitState } from "../../../e2e/support";
import { captionOf } from "../src/captions";
import { parseJsonc } from "../src/jsonc";

const INSTALL = /orbe add|cargo install|github\.com\/orbefab\/embedded|orbefab\.com\/embedded\/install\.sh/;
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
  const page = await context.newPage();
  const tracked = track(page);
  await instrument(page);
  await page.goto(url);
  return { context, page, ...tracked };
}

const bodyText = (page: Page) => page.evaluate(() => document.body.innerText);

function licences(page: Page): Promise<Record<string, string | null>> {
  return page
    .locator("section", { has: page.locator("h2", { hasText: /^Files$/ }) })
    .locator("tbody tr")
    .evaluateAll((rows) => Object.fromEntries(rows.map((row) => [row.cells[0].textContent, row.cells[2].textContent])));
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
  let weight: () => Promise<Weight> = async () => ({ requests: 0, bytes: 0, gzip: 0, wasm: false });

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors, urls, weight } = await openTracked(browser, `${host.base}/embedded/`));
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the landing page says what orbe is", async () => {
    expect(await page.textContent("h1")).toBe("orbe");
  });

  test("the first Block plays on the landing page", async () => {
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 30_000 });
    await page.waitForFunction(() => performance.getEntriesByName("orbe:first-frame:nano-button-led").length > 0, null, { timeout: 30_000 });
    await waitState(page, "playing");
    expect(await page.locator('[data-testid="state"]').textContent()).toBe("playing");
    await expect(page.getByTestId("block-caption")).toHaveText(captionOf("nano-button-led"));
    await expect(page.getByTestId("hold-to-press")).toHaveText("Hold to press");
    expect(await page.getByTestId("block-stage").getAttribute("tabindex")).toBeNull();
  });

  test("the landing page shows no install command", async () => {
    const first = await page.evaluate(
      (name) => performance.getEntriesByName(name)[0]?.startTime ?? null,
      "orbe:first-frame:nano-button-led",
    );
    await page.waitForTimeout(500);
    const loaded = await weight();
    console.log(
      `     landing: first Block frame at ${Math.round(first ?? 0)} ms; ${loaded.requests} requests, ${loaded.bytes} bytes, ${loaded.gzip} gzipped, the Worker's wasm ${loaded.wasm ? "included" : "missing"}`,
    );
    const body = await bodyText(page);
    expect(INSTALL.test(body), body.match(INSTALL)?.[0] ?? "").toBe(false);
  });

  test("it says the CLI is coming", async () => {
    expect(await page.locator('[data-testid="cli-coming"]').count()).toBe(1);
    await shot(page, "site-landing");
  });

  test("a card counts the parts the scene places", async () => {
    const counts = await page.locator('a[href^="/embedded/blocks/"]').evaluateAll((links) =>
      Object.fromEntries(
        links.flatMap((link) => {
          const count = link.querySelector("span.font-mono")?.textContent;
          return count?.endsWith(" parts") ? [[link.getAttribute("href"), count]] : [];
        }),
      ),
    );
    expect(counts).toEqual({
      "/embedded/blocks/nano-button-led/": "nano-board, 6 parts",
      "/embedded/blocks/nano-led-fade/": "nano-board, 4 parts",
      "/embedded/blocks/nano-light-comparator/": "nano-board, 10 parts",
      "/embedded/blocks/uno-button-led/": "uno-board, 6 parts",
    });
  });

  test("switching to nano-led-fade terminates the first Worker and revokes every object URL it made", async () => {
    await page.click('[data-block="nano-led-fade"]');
    await page.waitForFunction(() => performance.getEntriesByName("orbe:first-frame:nano-led-fade").length > 0, null, { timeout: 30_000 });
    const freed = await page.evaluate(() => {
      const { workers, created, revoked } = window.orbeTrack;
      const [one, two] = workers;
      const before = created.slice(0, two.created);
      return {
        workers: workers.length,
        firstTerminated: one.terminated,
        secondTerminated: two.terminated,
        firstUrls: before.length,
        firstRevoked: before.filter((url) => revoked.has(url)).length,
        secondRevoked: created.slice(two.created).filter((url) => revoked.has(url)).length,
      };
    });
    expect(
      freed.workers === 2 &&
        freed.firstTerminated &&
        !freed.secondTerminated &&
        freed.firstUrls > 0 &&
        freed.firstRevoked === freed.firstUrls &&
        freed.secondRevoked === 0,
      JSON.stringify(freed),
    ).toBe(true);
  });

  test("nano-led-fade plays after the switch", async () => {
    await waitState(page, "playing");
    expect(await page.locator('[data-testid="state"]').textContent()).toBe("playing");
    await expect(page.getByTestId("block-caption")).toHaveText(captionOf("nano-led-fade"));
    await expect(page.getByTestId("hold-to-press")).toHaveCount(0);
    await page.waitForTimeout(1500);
    await shot(page, "site-landing-switched");
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
    const files = await page.locator("section a[download]").evaluateAll((links) => links.map((link) => [link.textContent, link.getAttribute("href")]));
    expect(files.length === 5 && files.some(([path]) => path === "led-5mm.glb"), JSON.stringify(files)).toBe(true);
  });

  test("every file shows its licence", async () => {
    const shown = await page
      .locator("section", { has: page.locator("h2", { hasText: /^Files$/ }) })
      .locator("td:nth-child(3)")
      .allTextContents();
    expect(shown.length === 5 && shown.every((licence) => licence === "MIT OR Apache-2.0"), JSON.stringify(shown)).toBe(true);
  });

  test("a download serves the file's bytes, by the record's SHA-256", async () => {
    const href = await page.locator("section a[download]", { hasText: /^part\.jsonc$/ }).getAttribute("href");
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
    await page.evaluate(() => [...document.querySelectorAll("h2")].find((heading) => heading.textContent === "Files")?.scrollIntoView());
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
    expect(/Pins\s*Only its holes\./i.test(await bodyText(page))).toBe(true);
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
    expect(/Used by[\s\S]*Button and LED on a Nano/i.test(await bodyText(page))).toBe(true);
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

  test("the page shows the scene and the firmware source", async () => {
    const body = await bodyText(page);
    expect(/scene\.jsonc/.test(body) && /firmware\/button\/button\.ino/.test(body)).toBe(true);
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
  const counts = await page.locator('a[href^="/embedded/blocks/"]').evaluateAll((links) =>
    Object.fromEntries(
      links.flatMap((link) => {
        const count = link.querySelector("span.font-mono")?.textContent;
        return count?.endsWith(" parts") ? [[link.getAttribute("href"), count]] : [];
      }),
    ),
  );
  expect(counts).toEqual({
    "/embedded/blocks/nano-button-led/": "nano-board, 6 parts",
    "/embedded/blocks/nano-led-fade/": "nano-board, 4 parts",
    "/embedded/blocks/nano-light-comparator/": "nano-board, 10 parts",
    "/embedded/blocks/uno-button-led/": "uno-board, 6 parts",
  });
  await context.close();
});

test("a touch drag on empty stage space does not press the button", async ({ browser, host }) => {
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true });
  const page = await context.newPage();
  const errors = collectLogs(page, "errors");
  await page.goto(`${host.base}/embedded/`);
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
  const before = await page.evaluate(() => window.orbeViewer.get().camera.position.toArray() as number[]);
  const session = await context.newCDPSession(page);
  await session.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ ...spot, id: 1 }] });
  await session.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: spot.x + 110, y: spot.y - 70, id: 1 }] });
  await page.waitForTimeout(400);
  expect(await ledGlow(page)).toBeLessThan(0.05);
  const after = await page.evaluate(() => window.orbeViewer.get().camera.position.toArray() as number[]);
  expect(
    before.some((value, index) => Math.abs(value - (after[index] ?? value)) > 0.5),
    JSON.stringify({ before, after }),
  ).toBe(true);
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
        (await page.title()) === "Not found | orbe" &&
        (await bodyText(page)).includes("404 NOT FOUND") &&
        (await bodyText(page)).includes("Try the parts or the Blocks.") &&
        (await page.locator('a[href="/"]').textContent()) === "Back home" &&
        pageErrors.length === 0,
      JSON.stringify({ path, status: response?.status(), title: await page.title(), pageErrors }),
    ).toBe(true);
    await context.close();
  }
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
