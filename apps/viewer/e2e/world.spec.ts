import type { BrowserContext, Page } from "@playwright/test";
import { collectLogs, expect, headedSession, orbeJson, test, type OrbeEnvelope } from "../../../e2e/support";

const VIEW = { width: 1280, height: 720 };

/** `10000 * (10 / light)` ohm, at most 1 Mohm. 0 lx is the dark rail. */
function ldrOhm(light: number): number {
  if (light === 0) return 1_000_000;
  return Math.min(1_000_000, 10_000 * (10 / light));
}

/** The divider's middle, 5 V over 10 kohm and the LDR. */
function mid(light: number): number {
  const ohm = ldrOhm(light);
  return (5 * ohm) / (10_000 + ohm);
}

function pinVoltage(envelope: OrbeEnvelope): number | undefined {
  const value = envelope.data?.value as { v?: number } | undefined;
  return value?.v;
}

function near(found: number, expected: number): boolean {
  return Math.abs(found - expected) <= 1e-6 * Math.abs(expected);
}

test.describe("light divider", () => {
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

  test("the page shows a light slider at 300 lx", async () => {
    expect(session.ok, JSON.stringify(session.error)).toBe(true);
    await page.goto(session.url ?? "");
    await page.waitForSelector('[data-testid="world-light"]', { timeout: 30_000 });
    expect(await page.getAttribute('[data-testid="world-light-value"]', "data-frame")).toBe("300");
    const before = orbe("get", "@ldr.a");
    const voltage = pinVoltage(before);
    expect({
      ok: before.ok,
      close: voltage !== undefined && near(voltage, mid(300)),
    }).toEqual({ ok: true, close: true });
  });

  test("releasing the slider at 0 lx is on the next frame, and the divider follows", async () => {
    await page.click('[data-testid="play"]');
    await page.locator('[data-testid="world-light"]').fill("0");
    await page.waitForFunction(
      () => document.querySelector('[data-testid="world-light-value"]')?.getAttribute("data-frame") === "0",
      null,
      { timeout: 10_000 },
    );
    const got = orbe("get", "@ldr.a");
    const voltage = pinVoltage(got);
    expect({
      ok: got.ok,
      frame: await page.getAttribute('[data-testid="world-light-value"]', "data-frame"),
      close: voltage !== undefined && near(voltage, mid(0)),
    }).toEqual({ ok: true, frame: "0", close: true });
    await page.screenshot({ path: "/tmp/d63-slider.png" });
  });

  test("the page refused no set", async () => {
    const errors = await page.locator('[data-testid="errors"]').textContent();
    expect(errors ?? "").not.toContain("set ");
    expect(logs.some((line) => line.startsWith("pageerror"))).toBe(false);
  });

  test("`orbe close` ends the session", async () => {
    const closed = orbe("close");
    await page.waitForSelector('[data-testid="closed"]', { timeout: 10_000 });
    expect(closed.ok).toBe(true);
  });
});
