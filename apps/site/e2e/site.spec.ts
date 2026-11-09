import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";
import type { BrowserContext, Page } from "@playwright/test";
import { collectLogs, expect, ledGlow, orbeJson, pointOn, REPO, shot, test, waitGlow, waitState } from "../../../e2e/support";

const INSTALL = /orbe add|cargo install|github\.com\/orbefab\/embedded/;
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

test.describe("nano-button-led", () => {
  let context: BrowserContext;
  let page: Page;
  let errors: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    ({ context, page, errors } = await openTracked(browser, `${host.base}/embedded/blocks/nano-button-led/`));
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the Block's page plays it", async () => {
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 30_000 });
    await waitState(page, "playing");
    expect(await page.locator('[data-testid="state"]').textContent()).toBe("playing");
  });

  test("held, the button lights @led1", async () => {
    await page.locator("canvas").scrollIntoViewIfNeeded();
    const stem = await pointOn(page, "@btn1", "stem", true);
    await page.mouse.move(stem.x, stem.y);
    await page.mouse.down();
    await waitGlow(page, true);
    expect(await ledGlow(page)).toBeGreaterThan(0.5);
    await shot(page, "site-block-pressed-lit");
  });

  test("released, @led1 goes dark", async () => {
    await page.mouse.up();
    await waitGlow(page, false);
    expect(await ledGlow(page)).toBeLessThan(0.05);
  });

  test("the page shows the scene and the firmware source", async () => {
    const body = await bodyText(page);
    expect(/scene\.jsonc/.test(body) && /firmware\/button\/button\.ino/.test(body)).toBe(true);
  });

  test("it shows no install command and no README", async () => {
    const body = await bodyText(page);
    expect(
      !INSTALL.test(body) && !/README\.md\n/.test(body.split("Files")[0] ?? ""),
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

  test("the session refused no command", async () => {
    expect(await page.locator('[data-testid="errors"]').count()).toBe(0);
  });

  test("the Block's page logged no error", () => {
    expect(errors, JSON.stringify(errors)).toEqual([]);
  });
});

test("an unknown path answers 404 with the site's page", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  const response = await page.goto(`${host.base}/embedded/no-such-page`);
  expect(response?.status() === 404 && (await page.textContent("h1")) === "Not here", String(response?.status())).toBe(true);
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
