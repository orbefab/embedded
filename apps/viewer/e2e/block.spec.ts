import type { BrowserContext, Page } from "@playwright/test";
import { collectLogs, expect, ledGlow, pointOn, shot, test, textOf, waitGlow, waitState } from "../../../e2e/support";

const VIEW = { width: 1280, height: 720 };

test.describe("nano-button-led", () => {
  let context: BrowserContext;
  let page: Page;
  let logs: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    context = await browser.newContext({ viewport: VIEW });
    page = await context.newPage();
    logs = collectLogs(page, "all");
    await page.goto(`${host.base}/?block=nano-button-led`);
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 30_000 });
    await waitState(page, "paused");
    await page.waitForTimeout(800);
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the page draws nano-button-led from the registry, paused at t = 0", async () => {
    expect(await textOf(page, "time")).toBe("t = 0.000000 s");
  });

  test("@led1's and @btn1's models load from the registry's files", async () => {
    const loaded = await page.evaluate(() => {
      const { scene } = window.orbeViewer.get();
      return scene.getObjectByName("@led1")?.getObjectByName("body") !== undefined && scene.getObjectByName("@btn1")?.getObjectByName("stem") !== undefined;
    });
    expect(loaded, String(await textOf(page, "errors"))).toBe(true);
  });

  test("the fixture's test hook is absent on a Block's page", async () => {
    expect(await page.evaluate(() => window.orbeFixture === undefined)).toBe(true);
    await shot(page, "block-opened");
  });

  test("play runs the Worker's session past t = 0", async () => {
    await page.click('[data-testid="play"]');
    await waitState(page, "playing");
    await page.waitForTimeout(500);
    const time = Number((await textOf(page, "time"))?.match(/t = ([0-9.]+) s/)?.[1]);
    expect(time, String(await textOf(page, "time"))).toBeGreaterThan(0);
  });

  test("held, @led1 glows past 0.5", async () => {
    const stem = await pointOn(page, "@btn1", "stem");
    await page.mouse.move(stem.x, stem.y);
    await page.mouse.down();
    await waitGlow(page, true);
    expect(await ledGlow(page)).toBeGreaterThan(0.5);
    await shot(page, "block-pressed-lit");
  });

  test("released, @led1 goes dark", async () => {
    await page.mouse.up();
    await waitGlow(page, false);
    expect(await ledGlow(page)).toBeLessThan(0.05);
    await shot(page, "block-released");
  });

  test("the session refused no command the page sent, and every model loaded", async () => {
    expect(await textOf(page, "errors")).toBeNull();
  });

  test("the page logged no error", () => {
    expect(logs.some((line) => line.startsWith("pageerror"))).toBe(false);
  });
});

test.describe("missing block", () => {
  let context: BrowserContext;
  let page: Page;
  let logs: string[] = [];

  test.beforeAll(async ({ browser, host }) => {
    context = await browser.newContext({ viewport: VIEW });
    page = await context.newPage();
    logs = collectLogs(page, "all");
    await page.goto(`${host.base}/?block=no-such-block`);
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("a Block the registry lacks ends the session and says why", async () => {
    await page.waitForSelector('[data-testid="closed"]', { timeout: 10_000 });
    expect(await textOf(page, "closed")).toBe("the session ended: the Block did not load: the registry has no no-such-block");
  });

  test("the page logged no error", () => {
    expect(logs.some((line) => line.startsWith("pageerror"))).toBe(false);
  });
});
