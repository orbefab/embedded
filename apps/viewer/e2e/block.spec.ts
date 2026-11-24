import type { BrowserContext, Page } from "@playwright/test";
import { collectLogs, expect, repoBlocks, shot, showBlock, test, textOf, waitState } from "../../../e2e/support";

const VIEW = { width: 1280, height: 720 };

for (const block of repoBlocks()) {
  test.describe(block.id, () => {
    let context: BrowserContext;
    let page: Page;
    let logs: string[] = [];

    test.beforeAll(async ({ browser, host }) => {
      context = await browser.newContext({ viewport: VIEW });
      page = await context.newPage();
      logs = collectLogs(page, "all");
      await page.goto(`${host.base}/?block=${block.id}`);
      await page.waitForFunction(
        (kind) => {
          const scene = window.orbeViewer?.get().scene;
          if (kind === "relay") {
            return scene?.getObjectByName("@relay.led")?.getObjectByName("body") !== undefined
              && scene?.getObjectByName("@led1")?.getObjectByName("body") !== undefined;
          }
          if (kind === "digit") return scene?.getObjectByName("@disp")?.getObjectByName("seg-a") !== undefined;
          if (kind === "pixels") return scene?.getObjectByName("@stick.px1")?.getObjectByName("lens") !== undefined;
          if (kind === "servo") return scene?.getObjectByName("@servo")?.getObjectByName("horn") !== undefined;
          if (kind === "screen") return scene?.getObjectByName("@oled.display")?.getObjectByName("glass") !== undefined;
          if (kind === "lcd") return scene?.getObjectByName("@lcd.display")?.getObjectByName("glass") !== undefined;
          if (kind === "rgb") {
            return scene?.getObjectByName("@board.led")?.getObjectByName("lens") !== undefined
              && scene?.getObjectByName("@btn1")?.getObjectByName("stem") !== undefined;
          }
          const led = scene?.getObjectByName("@led1")?.getObjectByName("body") !== undefined;
          const button = kind !== "button" || scene?.getObjectByName("@btn1")?.getObjectByName("stem") !== undefined;
          return led && button;
        },
        block.kind,
        { timeout: 120_000 },
      );
      await waitState(page, "paused");
    });

    test.afterAll(async () => {
      await context?.close();
    });

    test("opens paused in the Worker, plays, and shows what it does", async () => {
      expect(await textOf(page, "time")).toBe("t = 0.000000 s");
      expect(await page.locator('[data-testid="world"]').count()).toBe(block.kind === "light" ? 1 : 0);
      expect(await page.evaluate(() => !("orbeFixture" in window))).toBe(true);
      await shot(page, `${block.id}-opened`);
      await showBlock(page, block, false, null);
      expect(await textOf(page, "errors")).toBeNull();
    });

    test("the page logged no error", () => {
      expect(logs.some((line) => line.startsWith("pageerror"))).toBe(false);
    });
  });
}

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

test.describe("speed indicator", () => {
  test("shows the speed ratio while playing below 1x and hides it at 1x and when paused", async ({ browser, host }) => {
    const context = await browser.newContext({ viewport: VIEW });
    const page = await context.newPage();
    const logs = collectLogs(page, "all");
    try {
      let send: ((message: unknown) => void) | null = null;
      await page.routeWebSocket(/\/session\?t=/, (ws) => {
        send = (message) => ws.send(JSON.stringify(message));
        send({
          type: "scene",
          session: "speed",
          scene_hash: "speed",
          instances: [],
          parts: {},
          wires: [],
          channels: { horns: [], leds: [], pins: [], pixels: [], switches: [] },
        });
        send({
          type: "frame",
          seq: 0,
          t_ps: 100_000_000_000,
          state: "playing",
          pins: [],
          led_a: [],
          switches: [],
          pixel_rgb: [],
          ratio: 0.07,
          diagnostics: 0,
        });
      });

      await page.goto(`${host.base}/#t=${"0".repeat(64)}`);
      await waitState(page, "playing");
      expect(await textOf(page, "speed")).toBe("0.07x");

      send!({
        type: "frame",
        seq: 1,
        t_ps: 116_000_000_000,
        state: "playing",
        pins: [],
        led_a: [],
        switches: [],
        pixel_rgb: [],
        ratio: 1,
        diagnostics: 0,
      });
      await expect(page.locator('[data-testid="speed"]')).toHaveCount(0);

      send!({
        type: "frame",
        seq: 2,
        t_ps: 132_000_000_000,
        state: "playing",
        pins: [],
        led_a: [],
        switches: [],
        pixel_rgb: [],
        ratio: 0.42,
        diagnostics: 0,
      });
      await expect(page.locator('[data-testid="speed"]')).toHaveText("0.42x");

      send!({
        type: "frame",
        seq: 3,
        t_ps: 132_000_000_000,
        state: "paused",
        pins: [],
        led_a: [],
        switches: [],
        pixel_rgb: [],
        diagnostics: 0,
      });
      await waitState(page, "paused");
      await expect(page.locator('[data-testid="speed"]')).toHaveCount(0);
      expect(logs.some((line) => line.startsWith("pageerror"))).toBe(false);
    } finally {
      await context.close();
    }
  });
});
