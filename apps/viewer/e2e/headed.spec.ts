import type { BrowserContext, Page } from "@playwright/test";
import { collectLogs, expect, headedSession, ledGlow, orbeJson, pointOn, shot, test, textOf, waitState, type OrbeEnvelope } from "../../../e2e/support";

/** led-5mm's `readings.brightness.full`, the current the viewer draws at glow 1. */
const LED_FULL_A = 0.01;
const VIEW = { width: 1280, height: 720 };

test.describe("live session", () => {
  let context: BrowserContext;
  let page: Page;
  let logs: string[] = [];
  let env: NodeJS.ProcessEnv = process.env;
  let session = { ok: false, url: null as string | null, error: null as unknown, runtime: "" };

  const orbe = (...args: string[]): OrbeEnvelope => orbeJson(process.env.ORBE ?? "", args, env);

  test.beforeAll(async ({ browser }) => {
    session = headedSession();
    env = { ...process.env, XDG_RUNTIME_DIR: session.runtime };
    delete env.ORBE_SESSION;
    delete env.ORBE_VIEWER_DIR;
    context = await browser.newContext({ viewport: VIEW });
    page = await context.newPage();
    logs = collectLogs(page, "all");
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("`orbe open --headed --no-browser` opens the scene and gives the page's URL", () => {
    const url = session.url ?? "";
    expect(session.ok && /^http:\/\/127\.0\.0\.1:\d+\/#t=[0-9a-f]{64}$/.test(url), JSON.stringify({
      ok: session.ok,
      error: session.error,
      url: url.replace(/#t=.*/, "#t=<token>"),
    })).toBe(true);
  });

  test("the page draws the session's scene, paused at t = 0", async () => {
    await page.goto(session.url ?? "");
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 30_000 });
    await waitState(page, "paused");
    await page.waitForTimeout(800);
    expect(await textOf(page, "time")).toBe("t = 0.000000 s");
    expect(await page.locator('[data-testid="world"]').count()).toBe(0);
    await shot(page, "headed-opened");
  });

  test("window.orbeFixture is absent", async () => {
    expect(await page.evaluate(() => !("orbeFixture" in window))).toBe(true);
  });

  test("play runs the session: `orbe get` sees it playing past t = 0", async () => {
    await page.click('[data-testid="play"]');
    await waitState(page, "playing");
    await page.waitForTimeout(500);
    const before = orbe("get", "@led1");
    expect({ state: before.state, past: (before.t_ps ?? 0) > 0 }).toEqual({ state: "playing", past: true });
    await shot(page, "headed-playing");
  });

  test("held and paused, `orbe get @led1` reads lit at the current the viewer glows at", async () => {
    const stem = await pointOn(page, "@btn1", "stem");
    await page.mouse.move(stem.x, stem.y);
    await page.mouse.down();
    await page.waitForFunction(
      () => window.orbeViewer.get().scene.getObjectByName("@led1").getObjectByName("body").material.emissiveIntensity > 0.5,
      null,
      { timeout: 10_000 },
    );
    await page.waitForTimeout(300);
    await shot(page, "headed-pressed-lit");
    const paused = orbe("pause");
    await waitState(page, "paused");
    await page.waitForTimeout(200);
    const lit = orbe("get", "@led1");
    const drawn = await ledGlow(page);
    const read = (lit.data?.value?.i ?? 0) / LED_FULL_A;
    expect({
      paused: paused.ok,
      state: lit.data?.value?.state,
      close: Math.abs(drawn - read) <= 0.05 * read,
    }).toEqual({ paused: true, state: "lit", close: true });
    await shot(page, "headed-paused-lit");
  });

  test("released and playing, @led1 goes dark and `orbe get` reads it off", async () => {
    await page.mouse.up();
    await page.click('[data-testid="play"]');
    await waitState(page, "playing");
    await page.waitForFunction(
      () => window.orbeViewer.get().scene.getObjectByName("@led1").getObjectByName("body").material.emissiveIntensity < 0.05,
      null,
      { timeout: 10_000 },
    );
    const off = orbe("get", "@led1");
    expect(off.data?.value?.state).toBe("off");
    await shot(page, "headed-released");
  });

  test("the session refused no command the page sent", async () => {
    expect(await textOf(page, "errors")).toBeNull();
  });

  test("the page logged no error", () => {
    expect(logs.some((line) => line.startsWith("pageerror"))).toBe(false);
  });

  test("`orbe close` ends the page's session and the page says why", async () => {
    const closed = orbe("close");
    await page.waitForSelector('[data-testid="closed"]', { timeout: 10_000 });
    expect({ ok: closed.ok, why: await textOf(page, "closed") }).toEqual({
      ok: true,
      why: "the session ended: `orbe close` ended the session",
    });
    await shot(page, "headed-closed");
  });
});
