import type { BrowserContext, Page } from "@playwright/test";
import { collectLogs, expect, headedSession, orbeJson, shot, test, waitGlow, waitState, type OrbeEnvelope } from "../../../e2e/support";

const VIEW = { width: 1280, height: 720 };
const THUMB = '[data-testid="param-pot1-position"]';
const FIELD = '[data-testid="param-pot1-position-value"]';

/** The turn of `@pot1`'s shaft from where the model draws it, in whole degrees clockwise seen from
 * the shaft, which is the joint's axis -Z. */
function knobDegrees(page: Page): Promise<number> {
  return page.evaluate(() => {
    const shaft = window.orbeViewer.get().scene.getObjectByName("@pot1")?.getObjectByName("shaft");
    if (shaft === undefined) throw new Error("no @pot1 shaft");
    const q = shaft.quaternion;
    return Math.round((2 * Math.atan2(-q.z, q.w) * 180) / Math.PI);
  });
}

function value(envelope: OrbeEnvelope): unknown {
  return envelope.data?.value;
}

test.describe("knob", () => {
  let context: BrowserContext;
  let page: Page;
  let logs: string[] = [];
  let env: NodeJS.ProcessEnv = process.env;
  let session = { ok: false, url: null as string | null, error: null as unknown, runtime: "" };
  /** Every `set` the page sent, in order. */
  const sets: { ref?: string; value?: number }[] = [];

  const orbe = (...args: string[]): OrbeEnvelope => orbeJson(process.env.ORBE ?? "", args, env);

  /** Waits for the frame to show `position`, then for the knob to stand at its angle. */
  async function shown(position: number) {
    await page.waitForFunction(
      ([field, position]) => document.querySelector(field)?.getAttribute("data-frame") === position,
      [FIELD, String(position)] as const,
      { timeout: 10_000 },
    );
    const degrees = Math.round((position - 0.5) * 300);
    await page.waitForFunction(
      (degrees) => {
        const q = window.orbeViewer.get().scene.getObjectByName("@pot1")?.getObjectByName("shaft")?.quaternion;
        return q !== undefined && Math.round((2 * Math.atan2(-q.z, q.w) * 180) / Math.PI) === degrees;
      },
      degrees,
      { timeout: 10_000 },
    );
    return { field: await page.locator(FIELD).inputValue(), knob: await knobDegrees(page) };
  }

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
        const message = JSON.parse(String(payload)) as { cmd?: string; ref?: string; value?: number };
        if (message.cmd === "set") sets.push({ ref: message.ref, value: message.value });
      }),
    );
  });

  test.afterAll(async () => {
    await context?.close();
  });

  test("the page opens paused with a slider at the scene's position and the knob turned to it", async () => {
    expect(session.ok, JSON.stringify(session.error)).toBe(true);
    await page.goto(session.url ?? "");
    await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@pot1") !== undefined, null, { timeout: 30_000 });
    await page.waitForSelector(THUMB, { timeout: 30_000 });
    await waitState(page, "paused");
    expect({ label: await page.locator('label[for="param-pot1-position"]').textContent(), ...(await shown(0.2)) }).toEqual({
      label: "pot1.position",
      field: "0.2",
      knob: -90,
    });
    await shot(page, "knob-opened");
  });

  test("`orbe step 50ms` runs past the sketch's first reading, and the LED stays off", async () => {
    const stepped = orbe("step", "50ms");
    await waitGlow(page, false);
    expect({ ok: stepped.ok, position: value(orbe("get", "@pot1.position")) }).toEqual({ ok: true, position: 0.2 });
  });

  test("typing 0.8 sends one set like `orbe set @pot1.position 0.8`, the knob turns and the LED lights", async () => {
    await page.click('[data-testid="play"]');
    await waitState(page, "playing");
    const from = sets.length;
    const field = page.locator(FIELD);
    await field.click();
    await field.fill("0.8");
    await field.press("Enter");
    const turned = await shown(0.8);
    await waitGlow(page, true);
    expect({ sent: sets.slice(from), ...turned, cli: value(orbe("get", "@pot1.position")) }).toEqual({
      sent: [{ ref: "@pot1.position", value: 0.8 }],
      field: "0.8",
      knob: 90,
      cli: 0.8,
    });
    await shot(page, "knob-turned");
  });

  test("`orbe pause` lists what the page's turn did while playing", async () => {
    const paused = orbe("pause");
    const data = paused.data as
      | { changes?: { kind: string; ref?: string; level?: string }[]; leds?: { ref: string; state: string }[] }
      | undefined;
    await waitState(page, "paused");
    expect({
      ok: paused.ok,
      d13: (data?.changes ?? []).some((change) => change.kind === "edge" && change.ref === "@nano.D13" && change.level === "high"),
      led1: data?.leds?.find((led) => led.ref === "@led1")?.state,
    }).toEqual({ ok: true, d13: true, led1: "lit" });
  });

  test("`orbe set @pot1.position 0.3` shows on the paused page, and the LED goes out once time runs", async () => {
    const from = sets.length;
    const set = orbe("set", "@pot1.position", "0.3");
    const turned = await shown(0.3);
    expect(orbe("step", "100ms").ok).toBe(true);
    await waitGlow(page, false);
    expect({ ok: set.ok, sent: sets.slice(from), ...turned }).toEqual({ ok: true, sent: [], field: "0.3", knob: -60 });
  });

  test("five quick ArrowRight presses send one set, five hundredths up", async () => {
    const from = sets.length;
    await page.locator(THUMB).focus();
    for (let k = 0; k < 5; k += 1) await page.keyboard.press("ArrowRight");
    const turned = await shown(0.35);
    await page.waitForTimeout(800);
    expect({ sent: sets.slice(from), ...turned }).toEqual({ sent: [{ ref: "@pot1.position", value: 0.35 }], field: "0.35", knob: -45 });
  });

  test("a position out of range sends nothing and says why, as orbe set does", async () => {
    const from = sets.length;
    const field = page.locator(FIELD);
    await field.click();
    await field.fill("1.5");
    await field.press("Enter");
    const why = await page.locator('[data-testid="param-pot1-position-error"]').textContent();
    const cli = orbe("set", "@pot1.position", "1.5");
    await field.press("Escape");
    expect({ why, sent: sets.slice(from), code: cli.error?.code, message: cli.error?.message }).toEqual({
      why: "pot1.position is above 0 and below 1, not 1.5",
      sent: [],
      code: "value",
      message: "pot1.position is above 0 and below 1, not 1.5",
    });
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
