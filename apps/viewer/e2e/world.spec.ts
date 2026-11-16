import type { BrowserContext, Page } from "@playwright/test";
import { collectLogs, expect, headedSession, orbeJson, shot, test, waitState, type OrbeEnvelope } from "../../../e2e/support";

const VIEW = { width: 1280, height: 720 };
const THUMB = '[data-testid="world-light"]';
const FIELD = '[data-testid="world-light-value"]';
/** The values the light thumb steps through. */
const ROUND_LUX = [0, 1, 2, 3, 5, 10, 20, 30, 50, 100, 200, 300, 500, 1000, 2000, 3000, 5000, 10_000, 20_000, 30_000, 50_000, 100_000];

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

function near(found: number | undefined, expected: number): boolean {
  return found !== undefined && Math.abs(found - expected) <= 1e-6 * Math.abs(expected);
}

test.describe("light divider", () => {
  let context: BrowserContext;
  let page: Page;
  let logs: string[] = [];
  let env: NodeJS.ProcessEnv = process.env;
  let session = { ok: false, url: null as string | null, error: null as unknown, runtime: "" };
  /** The value of every `set` the page sent, in order. */
  const sets: number[] = [];

  const orbe = (...args: string[]): OrbeEnvelope => orbeJson(process.env.ORBE ?? "", args, env);

  /** The `set`s sent since `from`, once the frame shows `light` and nothing more was sent for a
   * while longer than the slider waits after a key. */
  async function landed(from: number, light: number) {
    await page.waitForFunction(
      ([field, light]) => document.querySelector(field)?.getAttribute("data-frame") === light,
      [FIELD, String(light)] as const,
      { timeout: 10_000 },
    );
    await page.waitForTimeout(800);
    const field = page.locator(FIELD);
    return {
      sent: sets.slice(from),
      frame: await field.getAttribute("data-frame"),
      field: await field.inputValue(),
      thumb: await page.locator(THUMB).getAttribute("aria-valuetext"),
      divider: near(pinVoltage(orbe("get", "@ldr.a")), mid(light)),
    };
  }

  const shown = (light: string) => ({ frame: light, field: light, thumb: `${light} lx`, divider: true });

  test.beforeAll(async ({ browser }) => {
    session = headedSession();
    env = { ...process.env, XDG_RUNTIME_DIR: session.runtime };
    delete env.ORBE_SESSION;
    delete env.ORBE_VIEWER_DIR;
    context = await browser.newContext({ viewport: VIEW });
    page = await context.newPage();
    logs = collectLogs(page, "all");
    page.on("websocket", (socket) =>
      socket.on("framesent", ({ payload }) => {
        const message = JSON.parse(String(payload)) as { cmd?: string; value?: number };
        if (message.cmd === "set" && message.value !== undefined) sets.push(message.value);
      }),
    );
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the page opens paused with the light slider at the scene's 300 lx", async () => {
    expect(session.ok, JSON.stringify(session.error)).toBe(true);
    await page.goto(session.url ?? "");
    await page.waitForSelector(THUMB, { timeout: 30_000 });
    await waitState(page, "paused");
    expect(await landed(0, 300)).toEqual({ sent: [], ...shown("300") });
    await shot(page, "world-opened");
  });

  test("`orbe set light 50` shows on the paused page, and the divider follows once it plays", async () => {
    const from = sets.length;
    const set = orbe("set", "light", "50");
    await page.waitForFunction((field) => document.querySelector(field)?.getAttribute("data-frame") === "50", FIELD, { timeout: 10_000 });
    const paused = await page.locator('[data-testid="state"]').textContent();
    await shot(page, "world-cli-set");
    await page.click('[data-testid="play"]');
    await waitState(page, "playing");
    expect({ ok: set.ok, paused, ...(await landed(from, 50)) }).toEqual({ ok: true, paused: "paused", sent: [], ...shown("50") });
  });

  test("five quick ArrowRight presses send one set, five round values up, to 1000 lx", async () => {
    const from = sets.length;
    await page.locator(THUMB).focus();
    for (let k = 0; k < 5; k += 1) await page.keyboard.press("ArrowRight");
    expect(await landed(from, 1000)).toEqual({ sent: [1000], ...shown("1000") });
  });

  test("Home sends 0 lx once", async () => {
    const from = sets.length;
    await page.keyboard.press("Home");
    expect(await landed(from, 0)).toEqual({ sent: [0], ...shown("0") });
  });

  test("ArrowRight held for 15 repeats sends one set, fifteen round values up, to 3000 lx", async () => {
    const from = sets.length;
    for (let k = 0; k < 15; k += 1) {
      await page.keyboard.down("ArrowRight");
      await page.waitForTimeout(30);
    }
    await page.keyboard.up("ArrowRight");
    expect(await landed(from, 3000)).toEqual({ sent: [3000], ...shown("3000") });
  });

  test("End then four quick ArrowLeft send one set, 10000 lx", async () => {
    const from = sets.length;
    await page.keyboard.press("End");
    for (let k = 0; k < 4; k += 1) await page.keyboard.press("ArrowLeft");
    expect(await landed(from, 10_000)).toEqual({ sent: [10_000], ...shown("10000") });
  });

  test("a pointer drag to the right sends one set, when it lets go, of the round value under the thumb", async () => {
    const box = await page.locator(THUMB).boundingBox();
    if (box === null) throw new Error("the light thumb has no box");
    const from = sets.length;
    const y = box.y + box.height / 2;
    await page.mouse.move(box.x + box.width * 0.2, y);
    await page.mouse.down();
    for (let k = 1; k <= 12; k += 1) await page.mouse.move(box.x + box.width * (0.2 + k * 0.06), y);
    await page.waitForTimeout(500);
    const whileHeld = sets.length - from;
    await page.mouse.up();
    await page.waitForTimeout(800);
    const sent = sets.slice(from);
    const value = sent[0] ?? Number.NaN;
    const after = await landed(from, value);
    expect({ whileHeld, high: ROUND_LUX.includes(value) && value >= 20_000, ...after }).toEqual({ whileHeld: 0, high: true, sent: [value], ...shown(String(value)) });
  });

  test("typing 300 and Enter sends 300 lx once, and an out-of-range value sends nothing and says why", async () => {
    const from = sets.length;
    const field = page.locator(FIELD);
    await field.click();
    await field.fill("200000");
    await field.press("Enter");
    const why = await page.locator('[data-testid="world-light-error"]').textContent();
    await field.fill("300");
    await field.press("Enter");
    expect({ why, ...(await landed(from, 300)) }).toEqual({
      why: "light is from 0 to 100000 lx, not 200000",
      sent: [300],
      ...shown("300"),
    });
    await shot(page, "world-300");
  });

  test("the page refused no set", async () => {
    const errors = await page.evaluate(() => document.querySelector('[data-testid="errors"]')?.textContent ?? "");
    expect({
      refused: errors.split("\n").filter((line) => line.startsWith("set ")),
      pageErrors: logs.filter((line) => line.startsWith("pageerror")),
    }).toEqual({ refused: [], pageErrors: [] });
  });

  test("`orbe close` ends the session", async () => {
    const closed = orbe("close");
    await page.waitForSelector('[data-testid="closed"]', { timeout: 10_000 });
    expect(closed.ok).toBe(true);
  });
});
