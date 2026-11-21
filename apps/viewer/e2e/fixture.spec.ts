import type { Browser, BrowserContext, CDPSession, Page } from "@playwright/test";
import { collectLogs, expect, pointOn, rendersDir, shot, test } from "../../../e2e/support";

const VIEW = { width: 1280, height: 720 };
const near = (a: number[], b: number[], tolerance = 1e-3) => a.length === b.length && a.every((value, i) => Math.abs(value - b[i]) <= tolerance);

async function open(browser: Browser, base: string, { frame = "idle", touch = false } = {}) {
  const context = await browser.newContext({ viewport: VIEW, hasTouch: touch });
  const page = await context.newPage();
  const logs = collectLogs(page, "all");
  await page.goto(`${base}/?frame=${frame}`);
  await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 30_000 });
  await page.waitForTimeout(800);
  const commands = () => logs.filter((line) => /^(press|release) @\S+$/.test(line));
  const cdp = await context.newCDPSession(page);
  return { context, page, logs, commands, cdp };
}

/** The damping steps once per drawn frame, so frames, not wall time, say when the camera has stopped. */
async function still(page: Page) {
  const result = await page.evaluate(
    () =>
      new Promise((done) => {
        let last: number[] | undefined;
        let calm = 0;
        let frames = 0;
        const tick = () => {
          const { controls, camera } = window.orbeViewer.get();
          const now = [...controls.target.toArray(), camera.zoom];
          calm = last !== undefined && now.every((value, i) => Math.abs(value - last[i]) < 0.01) ? calm + 1 : 0;
          last = now;
          frames += 1;
          if (calm >= 5 || frames >= 600) done({ frames, calm, now });
          else requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      }),
  );
  const settled = result as { frames: number; calm: number; now: number[] };
  if (settled.calm < 5) throw new Error(`the camera still moved after ${settled.frames} frames: ${JSON.stringify(settled.now)}`);
}

async function toggleView(page: Page, type: string) {
  await page.keyboard.press("t");
  await page.waitForFunction((type) => {
    const { camera, controls } = window.orbeViewer.get();
    return camera.type === type && controls?.object === camera;
  }, type);
  await still(page);
}

const watchPointer = (page: Page) =>
  page.evaluate(() => {
    window.orbeViewer.get().gl.domElement.addEventListener("pointerdown", (event: PointerEvent) => {
      window.lastPointer = event.pointerId;
    });
  });

const dispatchPointer = (page: Page, type: string, init: Record<string, unknown>) =>
  page.evaluate(
    ([type, init]) => {
      const canvas = window.orbeViewer.get().gl.domElement;
      const before = canvas.hasPointerCapture(window.lastPointer);
      canvas.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: window.lastPointer, ...init }));
      return { before, after: canvas.hasPointerCapture(window.lastPointer) };
    },
    [type, init] as const,
  );

async function endListeners(cdp: CDPSession) {
  const { result } = await cdp.send("Runtime.evaluate", { expression: "window.orbeViewer.get().gl.domElement" });
  const { listeners } = await cdp.send("DOMDebugger.getEventListeners", { objectId: result.objectId });
  const by: Record<string, number> = {};
  for (const { type } of listeners) if (["pointerup", "pointermove", "pointercancel", "lostpointercapture"].includes(type)) by[type] = (by[type] ?? 0) + 1;
  return by;
}

const errorsShown = (page: Page) => page.evaluate(() => document.querySelector('[data-testid="errors"]')?.textContent ?? null);

const pointAt = (page: Page, ref: string, at: number[]) =>
  page.evaluate(
    ([ref, at]) => {
      const { scene, camera, size } = window.orbeViewer.get();
      const group = scene.getObjectByName(ref);
      const screen = group.localToWorld(group.position.clone().set(...at)).project(camera);
      return { x: ((screen.x + 1) / 2) * size.width, y: ((1 - screen.y) / 2) * size.height };
    },
    [ref, at] as const,
  );

const pointOnMat = (page: Page) =>
  page.evaluate(() => {
    const { scene, camera, size } = window.orbeViewer.get();
    let mat: any;
    scene.traverse((object: any) => {
      if (object.isMesh && object.geometry.type === "PlaneGeometry") mat = object;
    });
    const root = scene.getObjectByName("@nano").parent;
    const parts = root.children.map((group: any) => group.getWorldPosition(group.position.clone()));
    const right = Math.max(...parts.map((p: any) => p.x)) + 25;
    const at = mat.getWorldPosition(mat.position.clone());
    at.x = right;
    const screen = at.project(camera);
    return { x: ((screen.x + 1) / 2) * size.width, y: ((1 - screen.y) / 2) * size.height };
  });

const hitAt = (page: Page, at: { x: number; y: number }) =>
  page.evaluate(
    ([x, y]) => {
      const { scene, camera, size, raycaster, pointer } = window.orbeViewer.get();
      raycaster.setFromCamera(pointer.clone().set((x / size.width) * 2 - 1, 1 - (y / size.height) * 2), camera);
      const hit = raycaster.intersectObjects(scene.children, true)[0];
      if (hit === undefined) return null;
      if (hit.object.geometry.type === "PlaneGeometry") return "mat";
      for (let node = hit.object; node !== null; node = node.parent) if (node.userData.orbe) return node.userData.orbe.ref;
      return hit.object.type;
    },
    [at.x, at.y] as const,
  );

const label = (page: Page) =>
  page
    .locator('[data-testid="hover-label"]')
    .innerText({ timeout: 300 })
    .catch(() => null);

async function hoverUntil(page: Page, at: { x: number; y: number }, ref: string) {
  for (let r = 0; r <= 8; r += 1) {
    const ring = r === 0 ? [[0, 0]] : [[r, 0], [-r, 0], [0, r], [0, -r], [r, r], [-r, -r], [r, -r], [-r, r]];
    for (const [dx, dy] of ring) {
      await page.mouse.move(at.x + dx, at.y + dy);
      await page.waitForTimeout(40);
      if ((await label(page))?.split("\n")[0] === ref) return { x: at.x + dx, y: at.y + dy };
    }
  }
  return null;
}

async function click(page: Page, at: { x: number; y: number }) {
  await page.mouse.move(at.x, at.y);
  await page.mouse.down();
  await page.waitForTimeout(80);
  await page.mouse.up();
  await page.waitForTimeout(80);
}

const touch = (cdp: CDPSession, type: string, points: { x: number; y: number; id: number }[]) =>
  cdp.send("Input.dispatchTouchEvent", { type, touchPoints: points.map(({ x, y, id }) => ({ x, y, id })) });

const lookAt = (page: Page, ref: string, node: string, distance: number) =>
  page.evaluate(
    ([ref, node, distance]) => {
      const { scene, camera, controls } = window.orbeViewer.get();
      const target = scene.getObjectByName(ref).getObjectByName(node).getWorldPosition(camera.position.clone());
      const offset = camera.position.clone().sub(controls.target).setLength(distance);
      controls.target.copy(target);
      camera.position.copy(target).add(offset);
      controls.update();
    },
    [ref, node, distance] as const,
  );

const pushScene = (page: Page, edit: string) =>
  page.evaluate((source) => {
    const { source: fixture, firstTarget } = window.orbeFixture;
    fixture.push(new Function("scene", source)(firstTarget));
  }, edit);

const WORLD = [
  { name: "temperature", unit: "degC", min: -40, max: 125, start: 25, scale: { kind: "linear", step: 0.5 } },
  { name: "humidity", unit: "%RH", min: 0, max: 100, start: 50, scale: { kind: "linear", step: 1 } },
  { name: "light", unit: "lx", min: 0, max: 100_000, start: 300, scale: { kind: "log" } },
];

/** Opens the first target again with temperature, humidity and light declared, at 25 degC, 50 %RH
 * and 300 lx, the frame changed by `frame`. */
async function withWorld(page: Page, frame: Record<string, unknown> = {}) {
  await page.evaluate(
    ([world, frame]) => {
      const { source, firstTarget, script } = window.orbeFixture;
      source.push({ ...firstTarget, world });
      source.push({ ...script.idle, seq: 9, world: [25, 50, 300], ...frame });
    },
    [WORLD, frame] as const,
  );
  await page.waitForSelector('[data-testid="world-light"]');
}

/** Each slider's number, its thumb's spoken value, the frame's value and whether it is off. */
const worldShown = (page: Page) =>
  page.evaluate(() => {
    const rows: Record<string, { field: string; thumb: string | null; frame: string | null; disabled: boolean }> = {};
    for (const name of ["temperature", "humidity", "light"]) {
      const thumb = document.querySelector<HTMLInputElement>(`[data-testid="world-${name}"]`);
      const field = document.querySelector<HTMLInputElement>(`[data-testid="world-${name}-value"]`);
      if (thumb === null || field === null) continue;
      rows[name] = { field: field.value, thumb: thumb.getAttribute("aria-valuetext"), frame: field.getAttribute("data-frame"), disabled: thumb.disabled && field.disabled };
    }
    return rows;
  });

test.describe("fixture page", () => {
  test.afterAll(async ({ browser, host }) => {
    if (rendersDir() === undefined) return;
    for (const frame of ["idle", "pressed"]) {
      const { context, page } = await open(browser, host.base, { frame });
      await page.waitForTimeout(1200);
      await shot(page, frame);
      await lookAt(page, "@nano.led_pwr", "body", 30);
      await page.waitForTimeout(1200);
      await shot(page, `${frame}-nano-leds`);
      await context.close();
    }
  });

  test.describe("press and release", () => {
    let context: BrowserContext;
    let afterDown: string[] = [];
    let afterUp: string[] = [];
    let held: Record<string, number> = {};
    let released: Record<string, number> = {};
    let touchAction = "";

    test.beforeAll(async ({ browser, host }) => {
      const opened = await open(browser, host.base);
      context = opened.context;
      const { page, commands, cdp } = opened;
      const stem = await pointOn(page, "@btn1", "stem");
      await page.mouse.move(stem.x, stem.y);
      await page.mouse.down();
      await page.waitForTimeout(80);
      afterDown = commands();
      held = await endListeners(cdp);
      await page.mouse.up();
      await page.waitForTimeout(80);
      afterUp = commands();
      released = await endListeners(cdp);
      touchAction = await page.evaluate(() => getComputedStyle(document.querySelector('[data-testid="viewer"]')!).touchAction);
    });

    test.afterAll(async () => {
      await context?.close();
    });

    test("mouse down on btn1 presses it, and up releases it", () => {
      expect({ afterDown, afterUp }).toEqual({ afterDown: ["press @btn1"], afterUp: ["press @btn1", "release @btn1"] });
    });

    test("a press adds one pointerup, pointermove, pointercancel and lostpointercapture listener to the canvas, and its release removes them", () => {
      expect({ held, released }).toEqual({
        held: { pointerup: 1, pointermove: 1, pointercancel: 1, lostpointercapture: 1 },
        released: {},
      });
    });

    test("the viewer box's touch-action is none, so a finger on it never scrolls the page", () => {
      expect(touchAction).toBe("none");
    });
  });

  test("the right button alone on btn1 sends nothing", async ({ browser, host }) => {
    const { context, page, commands } = await open(browser, host.base);
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await page.mouse.move(stem.x, stem.y);
      await page.mouse.down({ button: "right" });
      await page.waitForTimeout(80);
      await page.mouse.up({ button: "right" });
      await page.waitForTimeout(80);
      expect(commands()).toEqual([]);
    } finally {
      await context.close();
    }
  });

  test("with the left button held on btn1, the right button going down and up does not release it, and the left up does", async ({ browser, host }) => {
    const { context, page, commands } = await open(browser, host.base);
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await watchPointer(page);
      await page.mouse.move(stem.x, stem.y);
      await page.mouse.down();
      await page.waitForTimeout(80);
      await page.mouse.down({ button: "right" });
      await page.mouse.up({ button: "right" });
      await page.waitForTimeout(80);
      const afterChord = commands();
      await dispatchPointer(page, "pointerup", { pointerType: "mouse", button: 2, buttons: 1 });
      await page.waitForTimeout(80);
      const afterRightUp = commands();
      await page.mouse.up();
      await page.waitForTimeout(80);
      expect({ afterChord, afterRightUp, afterLeftUp: commands() }).toEqual({
        afterChord: ["press @btn1"],
        afterRightUp: ["press @btn1"],
        afterLeftUp: ["press @btn1", "release @btn1"],
      });
    } finally {
      await context.close();
    }
  });

  test("in a chord on btn1, the left button lifting first releases it while the right is still down", async ({ browser, host }) => {
    const { context, page, commands } = await open(browser, host.base);
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await page.mouse.move(stem.x, stem.y);
      await page.mouse.down();
      await page.waitForTimeout(80);
      await page.mouse.down({ button: "right" });
      await page.waitForTimeout(80);
      await page.mouse.up();
      await page.waitForTimeout(80);
      const afterLeftUp = commands();
      await page.mouse.up({ button: "right" });
      await page.waitForTimeout(80);
      expect({ afterLeftUp, afterRightUp: commands() }).toEqual({
        afterLeftUp: ["press @btn1", "release @btn1"],
        afterRightUp: ["press @btn1", "release @btn1"],
      });
    } finally {
      await context.close();
    }
  });

  test("a press the session refuses and a release whose send rejects are both listed, and the next press works", async ({ browser, host }) => {
    const { context, page, commands } = await open(browser, host.base);
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await page.evaluate(() => {
        const { source } = window.orbeFixture;
        window.realSend = source.send;
        source.send = async (command: { cmd: string }) =>
          command.cmd === "press"
            ? { ok: false, session: "s", t_ps: 0, state: "paused", error: { code: "unknown_ref", message: "@btn1: no part named `btn1`", hint: "parts: nano" } }
            : Promise.reject(new Error("the session closed"));
      });
      await click(page, stem);
      const shown = await errorsShown(page);
      await page.evaluate(() => {
        window.orbeFixture.source.send = window.realSend;
      });
      await click(page, stem);
      expect({ shown, commands: commands() }).toEqual({
        shown: "press @btn1: error[unknown_ref]: @btn1: no part named `btn1`\n  hint: parts: nano\nrelease @btn1: the session closed",
        commands: ["press @btn1", "release @btn1"],
      });
    } finally {
      await context.close();
    }
  });

  test("a touch on btn1 that the browser cancels releases it", async ({ browser, host }) => {
    const { context, page, commands, cdp } = await open(browser, host.base, { touch: true });
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await touch(cdp, "touchStart", [{ ...stem, id: 1 }]);
      await page.waitForTimeout(150);
      const afterStart = commands();
      await touch(cdp, "touchCancel", []);
      await page.waitForTimeout(200);
      expect({ afterStart, afterCancel: commands() }).toEqual({
        afterStart: ["press @btn1"],
        afterCancel: ["press @btn1", "release @btn1"],
      });
    } finally {
      await context.close();
    }
  });

  test("a finger moving while it holds btn1 keeps the press, and its lift releases it", async ({ browser, host }) => {
    const { context, page, commands, cdp } = await open(browser, host.base, { touch: true });
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await touch(cdp, "touchStart", [{ ...stem, id: 1 }]);
      await page.waitForTimeout(80);
      for (let step = 1; step <= 5; step += 1) {
        await touch(cdp, "touchMove", [{ x: stem.x + 3 * step, y: stem.y + 2 * step, id: 1 }]);
        await page.waitForTimeout(40);
      }
      const afterMoves = commands();
      await touch(cdp, "touchEnd", []);
      await page.waitForTimeout(150);
      expect({ afterMoves, final: commands() }).toEqual({
        afterMoves: ["press @btn1"],
        final: ["press @btn1", "release @btn1"],
      });
    } finally {
      await context.close();
    }
  });

  test("a pointercancel on btn1's pointer releases it while the canvas still has the capture, and the lift sends nothing", async ({ browser, host }) => {
    const { context, page, commands, cdp } = await open(browser, host.base, { touch: true });
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await watchPointer(page);
      await touch(cdp, "touchStart", [{ ...stem, id: 1 }]);
      await page.waitForTimeout(150);
      const capture = await dispatchPointer(page, "pointercancel", { pointerType: "touch" });
      await page.waitForTimeout(150);
      const afterCancel = commands();
      await touch(cdp, "touchEnd", []);
      await page.waitForTimeout(150);
      expect({
        captured: capture.before && capture.after,
        afterCancel,
        final: commands(),
      }).toEqual({
        captured: true,
        afterCancel: ["press @btn1", "release @btn1"],
        final: ["press @btn1", "release @btn1"],
      });
    } finally {
      await context.close();
    }
  });

  test("losing the pointer capture releases btn1 before the up, and the up on the mat sends nothing", async ({ browser, host }) => {
    const { context, page, commands } = await open(browser, host.base);
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      const mat = await pointOnMat(page);
      await watchPointer(page);
      await page.mouse.move(stem.x, stem.y);
      await page.mouse.down();
      await page.mouse.move(stem.x + 1, stem.y);
      await page.waitForTimeout(80);
      const lost = await page.evaluate(() => {
        const canvas = window.orbeViewer.get().gl.domElement;
        const had = canvas.hasPointerCapture(window.lastPointer);
        canvas.releasePointerCapture(window.lastPointer);
        return had;
      });
      await page.mouse.move(mat.x, mat.y, { steps: 5 });
      await page.waitForTimeout(150);
      const afterLost = commands();
      await page.mouse.up();
      await page.waitForTimeout(150);
      expect({ lost, afterLost, final: commands(), upOn: await hitAt(page, mat) }).toEqual({
        lost: true,
        afterLost: ["press @btn1", "release @btn1"],
        final: ["press @btn1", "release @btn1"],
        upOn: "mat",
      });
    } finally {
      await context.close();
    }
  });

  test("a second finger on btn1 while it is held sends no second press, and lifting it does not release the first finger's", async ({ browser, host }) => {
    const { context, page, commands, cdp } = await open(browser, host.base, { touch: true });
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      const secondFingerOn = await hitAt(page, { x: stem.x + 1, y: stem.y });
      const f1 = { ...stem, id: 1 };
      const f2 = { x: stem.x + 1, y: stem.y, id: 2 };
      await touch(cdp, "touchStart", [f1]);
      await page.waitForTimeout(120);
      await touch(cdp, "touchStart", [f1, f2]);
      await page.waitForTimeout(120);
      // Chromium's touchEnd lifts the points it lists.
      await touch(cdp, "touchEnd", [f2]);
      await page.waitForTimeout(150);
      const afterSecondLifts = commands();
      await touch(cdp, "touchEnd", [f1]);
      await page.waitForTimeout(150);
      expect({ secondFingerOn, afterSecondLifts, afterFirstLifts: commands() }).toEqual({
        secondFingerOn: "@btn1",
        afterSecondLifts: ["press @btn1"],
        afterFirstLifts: ["press @btn1", "release @btn1"],
      });
    } finally {
      await context.close();
    }
  });

  test("the window losing focus releases btn1 once", async ({ browser, host }) => {
    const { context, page, commands } = await open(browser, host.base);
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await page.mouse.move(stem.x, stem.y);
      await page.mouse.down();
      await page.waitForTimeout(80);
      await page.evaluate(() => window.dispatchEvent(new Event("blur")));
      await page.waitForTimeout(80);
      const afterBlur = commands();
      await page.mouse.up();
      await page.waitForTimeout(80);
      expect({ afterBlur, afterUp: commands() }).toEqual({
        afterBlur: ["press @btn1", "release @btn1"],
        afterUp: ["press @btn1", "release @btn1"],
      });
    } finally {
      await context.close();
    }
  });

  test("a faulted frame shows its fault below the controls, and play is disabled", async ({ browser, host }) => {
    const { context, page } = await open(browser, host.base);
    try {
      const fault = "error[solver.convergence] Newton did not converge at #net.led1__anode after 10 iterations; the time step halved to 1 ps";
      await page.evaluate((fault) => {
        const { source, script } = window.orbeFixture;
        source.push({ ...script.idle, seq: 9, state: "faulted", diagnostics: 2, fault });
      }, fault);
      await page.waitForSelector('[data-testid="fault"]');
      const shown = await page.evaluate(() => ({
        state: document.querySelector('[data-testid="state"]')?.textContent ?? null,
        fault: document.querySelector('[data-testid="fault"]')?.textContent ?? null,
        play: (document.querySelector('[data-testid="play"]') as HTMLButtonElement | null)?.disabled ?? null,
      }));
      expect(shown).toEqual({ state: "faulted", fault, play: true });
      await shot(page, "fixture-faulted");
    } finally {
      await context.close();
    }
  });

  test("a scene's world quantities each get a slider at the frame's value", async ({ browser, host }) => {
    const { context, page } = await open(browser, host.base);
    try {
      await withWorld(page);
      expect(await worldShown(page)).toEqual({
        temperature: { field: "25", thumb: "25 degC", frame: "25", disabled: false },
        humidity: { field: "50", thumb: "50 %RH", frame: "50", disabled: false },
        light: { field: "300", thumb: "300 lx", frame: "300", disabled: false },
      });
      await shot(page, "fixture-world");
    } finally {
      await context.close();
    }
  });

  test("five quick presses on the light thumb send one set, and the thumb holds it until a frame shows it", async ({ browser, host }) => {
    const { context, page } = await open(browser, host.base);
    try {
      await withWorld(page);
      await page.evaluate(() => {
        const { source } = window.orbeFixture;
        window.sets = [];
        source.send = (command: { cmd: string; value: number }) =>
          new Promise((done) => {
            window.sets?.push(command);
            window.answer = () => done({ ok: true, session: "s", t_ps: 0, state: "paused", data: { quantity: "light", value: command.value, landed_ps: 1 } });
          });
      });
      await page.locator('[data-testid="world-light"]').focus();
      for (let k = 0; k < 5; k += 1) await page.keyboard.press("ArrowRight");
      await page.waitForTimeout(700);
      const pending = { sets: await page.evaluate(() => window.sets), light: (await worldShown(page)).light };
      await page.evaluate(() => window.answer?.());
      await page.waitForTimeout(100);
      const answered = (await worldShown(page)).light;
      await page.evaluate(() => window.orbeFixture.source.push({ ...window.orbeFixture.script.idle, seq: 10, world: [25, 50, 5000] }));
      await page.waitForTimeout(100);
      const framed = (await worldShown(page)).light;
      await page.evaluate(() => window.orbeFixture.source.push({ ...window.orbeFixture.script.idle, seq: 11, world: [25, 50, 20] }));
      await page.waitForTimeout(100);
      expect({ pending, answered, framed, elsewhere: (await worldShown(page)).light }).toEqual({
        pending: {
          sets: [{ cmd: "set", quantity: "light", value: 5000 }],
          light: { field: "5000", thumb: "5000 lx", frame: "300", disabled: false },
        },
        answered: { field: "5000", thumb: "5000 lx", frame: "300", disabled: false },
        framed: { field: "5000", thumb: "5000 lx", frame: "5000", disabled: false },
        elsewhere: { field: "20", thumb: "20 lx", frame: "20", disabled: false },
      });
    } finally {
      await context.close();
    }
  });

  test("a faulted session shows its fault clear of the world panel, with the sliders off and why", async ({ browser, host }) => {
    const { context, page } = await open(browser, host.base);
    try {
      const fault = "error[solver.convergence] Newton did not converge at #net.out after 10 iterations\n  at @u1\n  hint: check the feedback network";
      await withWorld(page, { state: "faulted", diagnostics: 1, fault });
      await page.waitForSelector('[data-testid="world-off"]');
      const boxes = await page.evaluate(() =>
        ["controls", "fault", "world"].map((id) => {
          const box = document.querySelector(`[data-testid="${id}"]`)?.getBoundingClientRect();
          return box === undefined ? null : { top: box.top, bottom: box.bottom };
        }),
      );
      const [controls, faultBox, world] = boxes;
      const shown = await worldShown(page);
      expect({
        stacked: controls !== null && faultBox !== null && world !== null && controls.bottom <= faultBox.top && faultBox.bottom <= world.top,
        off: Object.values(shown).every((row) => row.disabled),
        why: await page.locator('[data-testid="world-off"]').textContent(),
      }).toEqual({ stacked: true, off: true, why: "The session faulted, so the world stays as it is." });
      await shot(page, "fixture-world-faulted");
    } finally {
      await context.close();
    }
  });

  test("a press held across a new scene sends no release, and the next press works", async ({ browser, host }) => {
    const { context, page, commands } = await open(browser, host.base);
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await page.mouse.move(stem.x, stem.y);
      await page.mouse.down();
      await page.waitForTimeout(80);
      await pushScene(
        page,
        `return { ...scene, session: "reopened",
        instances: scene.instances.filter((i) => i.name !== "btn1"),
        wires: scene.wires.filter((w) => !w.ends.some((e) => e.startsWith("btn1."))),
        channels: { ...scene.channels, switches: ["@nano.sw_reset"] } };`,
      );
      await page.waitForFunction(() => window.orbeViewer.get().scene.getObjectByName("@btn1") === undefined);
      await page.waitForTimeout(800);
      await page.mouse.up();
      await page.waitForTimeout(100);
      const afterUp = commands();
      const reset = await hoverUntil(page, await pointOn(page, "@nano.sw_reset", "button"), "@nano.sw_reset");
      if (reset !== null) await click(page, reset);
      expect({ afterUp, final: commands() }).toEqual({
        afterUp: ["press @btn1"],
        final: ["press @btn1", "press @nano.sw_reset", "release @nano.sw_reset"],
      });
    } finally {
      await context.close();
    }
  });

  test.describe("clicks that send nothing, and reset", () => {
    let context: BrowserContext;
    let board: { x: number; y: number } | null = null;
    let matHit: string | null = null;
    let afterMat: string[] = [];
    let afterReset: string[] = [];

    test.beforeAll(async ({ browser, host }) => {
      const opened = await open(browser, host.base);
      context = opened.context;
      const { page, commands } = opened;
      board = await hoverUntil(page, await pointAt(page, "@nano", [3, -6, 4.2]), "@nano");
      if (board !== null) await click(page, board);
      const mat = await pointOnMat(page);
      matHit = await hitAt(page, mat);
      await click(page, mat);
      afterMat = commands();
      const reset = await hoverUntil(page, await pointOn(page, "@nano.sw_reset", "button"), "@nano.sw_reset");
      if (reset !== null) await click(page, reset);
      afterReset = commands();
    });

    test.afterAll(async () => {
      await context?.close();
    });

    test("clicks on the Nano's board and on the mat send nothing", () => {
      expect({ board: board !== null, matHit, commands: afterMat }).toEqual({ board: true, matHit: "mat", commands: [] });
    });

    test("a click on the Nano's reset button presses and releases @nano.sw_reset", () => {
      expect(afterReset).toEqual(["press @nano.sw_reset", "release @nano.sw_reset"]);
    });
  });

  test.describe("top view", () => {
    let context: BrowserContext;
    let page: Page;
    let afterUp = { target: [] as number[], zoom: 0 };

    test.beforeAll(async ({ browser, host }) => {
      const opened = await open(browser, host.base);
      context = opened.context;
      page = opened.page;
    });

    test.afterAll(async () => {
      await context?.close();
    });

    test("window.orbeViewer gives the live state", async () => {
      await toggleView(page, "OrthographicCamera");
      const live = await page.evaluate(() => {
        const viewer = window.orbeViewer;
        return { keys: Object.keys(viewer), camera: viewer.get().camera.type, stale: "camera" in viewer && viewer.camera !== viewer.get().camera };
      });
      expect(live).toEqual({ keys: ["get"], camera: "OrthographicCamera", stale: false });
    });

    test("the top view's screen-right is world +X and its screen-up world -Z, with camera.up -Z", async () => {
      const axes = await page.evaluate(() => {
        const { camera } = window.orbeViewer.get();
        const column = (i: number) => camera.matrixWorld.elements.slice(i * 4, i * 4 + 3).map((v: number) => +v.toFixed(4));
        return { up: camera.up.toArray(), right: column(0), screenUp: column(1) };
      });
      expect(near(axes.up, [0, 0, -1]) && near(axes.right, [1, 0, 0]) && near(axes.screenUp, [0, 0, -1]), JSON.stringify(axes)).toBe(true);
    });

    test("in the top view a left drag pans: dragging left moves the target +X, dragging up moves it +Z", async () => {
      await shot(page, "top");
      const state = () =>
        page.evaluate(() => {
          const { controls, camera } = window.orbeViewer.get();
          return { target: controls.target.toArray() as number[], zoom: camera.zoom as number };
        });
      const mat = await pointOnMat(page);
      const drag = async (dx: number, dy: number) => {
        const from = await state();
        await page.mouse.move(mat.x, mat.y);
        await page.mouse.down({ button: "left" });
        await page.mouse.move(mat.x + dx, mat.y + dy, { steps: 10 });
        await page.mouse.up({ button: "left" });
        await still(page);
        const to = await state();
        return { from, to, moved: to.target.map((value, i) => value - from.target[i]) };
      };
      const left = await drag(-200, 0);
      await toggleView(page, "PerspectiveCamera");
      await toggleView(page, "OrthographicCamera");
      const up = await drag(0, -150);
      afterUp = up.to;
      expect(left.moved[0] > 5 && Math.abs(left.moved[2]) < 0.5 && up.moved[2] > 5 && Math.abs(up.moved[0]) < 0.5, JSON.stringify({ left, up })).toBe(true);
    });

    test("in the top view the wheel zooms", async () => {
      await page.mouse.wheel(0, -400);
      await still(page);
      const afterWheel = await page.evaluate(() => window.orbeViewer.get().camera.zoom as number);
      expect(afterWheel > afterUp.zoom * 1.05, JSON.stringify({ before: afterUp.zoom, after: afterWheel })).toBe(true);
    });
  });

  test("in the top view a one-finger drag pans", async ({ browser, host }) => {
    const { context, page, cdp } = await open(browser, host.base, { touch: true });
    try {
      await toggleView(page, "OrthographicCamera");
      const mat = await pointOnMat(page);
      const target = () => page.evaluate(() => window.orbeViewer.get().controls.target.toArray() as number[]);
      const before = await target();
      await touch(cdp, "touchStart", [{ ...mat, id: 1 }]);
      for (let step = 1; step <= 10; step += 1) {
        await touch(cdp, "touchMove", [{ x: mat.x - step * 20, y: mat.y, id: 1 }]);
        await page.waitForTimeout(16);
      }
      await touch(cdp, "touchEnd", []);
      await still(page);
      const after = await target();
      expect(after[0] - before[0] > 5, JSON.stringify({ before, after })).toBe(true);
    } finally {
      await context.close();
    }
  });

  test.describe("hover reasons", () => {
    let context: BrowserContext;
    let page: Page;

    test.beforeAll(async ({ browser, host }) => {
      const opened = await open(browser, host.base);
      context = opened.context;
      page = opened.page;
    });

    test.afterAll(async () => {
      await context?.close();
    });

    test("hover on a not-simulated pin shows its reason", async () => {
      await lookAt(page, "@nano.j2", "pin-10", 45);
      await page.waitForTimeout(1200);
      const pin = await hoverUntil(page, await pointOn(page, "@nano.j2", "pin-10"), "@nano.A6");
      const text = await label(page);
      expect({ pin: pin !== null, text }).toEqual({
        pin: true,
        text: "@nano.A6\nnot simulated: The engine's atmega328p kind has the DIP's pins, with no ADC6 or ADC7 pad.",
      });
      await shot(page, "hover-a6");
    });

    test("hover on a not-simulated part shows its reason", async () => {
      await lookAt(page, "@nano.led_tx", "body", 45);
      await page.waitForTimeout(1200);
      const led = await hoverUntil(page, await pointOn(page, "@nano.led_tx", "body"), "@nano.led_tx");
      const text = await label(page);
      expect({ led: led !== null, text }).toEqual({
        led: true,
        text: "@nano.led_tx\nnot simulated: The CH340G drives it, and its model covers only TXD and DTR.",
      });
    });
  });

  test.describe("parts the viewer cannot draw as asked", () => {
    let context: BrowserContext;
    let page: Page;
    let logs: string[] = [];

    test.beforeAll(async ({ browser, host }) => {
      const opened = await open(browser, host.base);
      context = opened.context;
      page = opened.page;
      logs = opened.logs;
    });

    test.afterAll(async () => {
      await context?.close();
    });

    test("a 0.01 ohm resistor keeps its model's bands, warns once, and the scene still draws", async () => {
      const drawn = () =>
        page.evaluate(() => ({
          canvas: document.querySelectorAll("canvas").length,
          r1: window.orbeViewer.get().scene.getObjectByName("@r1") !== undefined,
          errors: document.querySelector('[data-testid="errors"]')?.textContent ?? null,
        }));
      await pushScene(
        page,
        `return { ...scene, session: "shunt",
        instances: scene.instances.map((i) => (i.name === "r1" ? { ...i, params: { resistance: 0.01 } } : i)) };`,
      );
      await page.waitForTimeout(800);
      const shunt = { ...(await drawn()), warnings: logs.filter((line) => line.includes("no 4-band code")).length };
      expect(shunt).toEqual({ canvas: 1, r1: true, errors: null, warnings: 1 });
    });

    test('a part whose view names a missing node is listed as an error, and the scene still draws', async () => {
      await pushScene(
        page,
        `const led = scene.parts["led-5mm"];
      return { ...scene, session: "bad-led",
        parts: { ...scene.parts, "led-5mm": { ...led, view: { ...led.view, glow: [{ node: "lens", by: "brightness" }] } } } };`,
      );
      await page.waitForTimeout(800);
      const bad = await page.evaluate(() => ({
        canvas: document.querySelectorAll("canvas").length,
        r1: window.orbeViewer.get().scene.getObjectByName("@r1") !== undefined,
        errors: document.querySelector('[data-testid="errors"]')?.textContent ?? null,
      }));
      expect(bad).toEqual({ canvas: 1, r1: true, errors: '@led1: led-5mm: the GLB has no node "lens"' });
    });
  });

  const STAGE_THEMES = {
    dark: { stage: "#17191c", mat: "#202327", line: "#2c3036" },
    light: { stage: "#e9e9ec", mat: "#e2e2e6", line: "#d2d2d8" },
  } as const;

  async function setStageTheme(page: Page, name: keyof typeof STAGE_THEMES) {
    const theme = STAGE_THEMES[name];
    await page.evaluate((theme) => {
      const root = document.documentElement;
      root.style.setProperty("--stage", theme.stage);
      root.style.setProperty("--stage-mat", theme.mat);
      root.style.setProperty("--stage-line", theme.line);
      root.classList.remove("light", "dark");
      root.classList.add(theme.stage === "#17191c" ? "dark" : "light");
    }, theme);
    await page.waitForFunction(
      (hex) => window.orbeViewer.get().scene.background?.getHexString?.() === hex,
      theme.stage.slice(1),
    );
  }

  /** The brightest pixel that is not an LED. A glowing body and a clear lens over it are hidden,
   * so a dim power LED does not count. An unlit lens and a white plastic still do. */
  function outsideLuminance(page: Page): Promise<number> {
    return page.evaluate(() => {
      const { scene, gl, camera, size } = window.orbeViewer.get();
      const { WebGLRenderTarget, FloatType } = window.orbeFixture.three;
      const glowing = (material: any) =>
        material?.emissiveIntensity > 0 && (material.emissive?.r > 0 || material.emissive?.g > 0 || material.emissive?.b > 0);
      const hidden: { visible: boolean }[] = [];
      const groups = new Set<any>();
      scene.traverse((object: any) => {
        if (!object.isMesh) return;
        const materials = Array.isArray(object.material) ? object.material : [object.material];
        if (materials.some(glowing)) groups.add(object.parent);
      });
      for (const group of groups) {
        group?.traverse((node: any) => {
          if (!node.isMesh) return;
          const materials = Array.isArray(node.material) ? node.material : [node.material];
          const glow = materials.some(glowing);
          const clear = materials.some((material: any) => material?.transparent || material?.opacity < 1);
          if (glow || clear) {
            hidden.push(node);
            node.visible = false;
          }
        });
      }
      const hdr = new WebGLRenderTarget(size.width, size.height, { type: FloatType });
      gl.setRenderTarget(hdr);
      gl.clear();
      gl.render(scene, camera);
      gl.setRenderTarget(null);
      const pixels = new Float32Array(size.width * size.height * 4);
      gl.readRenderTargetPixels(hdr, 0, 0, size.width, size.height, pixels);
      hdr.dispose();
      for (const node of hidden) node.visible = true;
      let max = 0;
      for (let i = 0; i < pixels.length; i += 4) {
        const y = 0.2126 * pixels[i]! + 0.7152 * pixels[i + 1]! + 0.0722 * pixels[i + 2]!;
        if (y > max) max = y;
      }
      return max;
    });
  }

  test("each 0603 LED takes its colour's tint, and the lit power LED glows in the red tint", async ({ browser, host }) => {
    const { context, page } = await open(browser, host.base);
    try {
      const colours = await page.evaluate(() => {
        const { scene } = window.orbeViewer.get();
        const material = (ref: string) => scene.getObjectByName(ref).getObjectByName("body").material;
        const ratio = (colour: { r: number; g: number; b: number }) => [colour.g / colour.r, colour.b / colour.r].map((v: number) => +v.toFixed(3));
        return {
          led_tx: `#${material("@nano.led_tx").color.getHexString()}`,
          led_rx: `#${material("@nano.led_rx").color.getHexString()}`,
          led_pwr: ratio(material("@nano.led_pwr").emissive),
          red: ratio(material("@nano.led_rx").color),
        };
      });
      expect(
        colours.led_tx === "#c5d6b0" && colours.led_rx === "#e0b0a4" && near(colours.led_pwr, colours.red),
        JSON.stringify(colours),
      ).toBe(true);
    } finally {
      await context.close();
    }
  });

  test("each LED's lit luminance is under 1 at 0.95 of its full current and within 3 % of 1 at full", async ({ browser, host }) => {
    const { context, page } = await open(browser, host.base);
    try {
      const glow = [];
      for (const theme of ["dark", "light"] as const) {
        await setStageTheme(page, theme);
        await page.evaluate((seq) => {
          const { source, script } = window.orbeFixture;
          source.push({ ...script.idle, seq });
        }, 800 + (theme === "dark" ? 0 : 1));
        await still(page);
        const idle = await outsideLuminance(page);
        expect(idle < 1, `${theme} idle outside luminance ${idle}`).toBe(true);
        for (const fraction of [0.95, 1.0]) {
          await page.evaluate((fraction) => {
            const { source, script } = window.orbeFixture;
            const amps = 0.01 * fraction;
            source.push({ ...script.idle, seq: 1000 + fraction, led_a: [amps, amps, amps] });
          }, fraction);
          await page.waitForTimeout(300);
          const row = await page.evaluate(({ fraction, theme }) => {
            const { scene, gl, camera, size } = window.orbeViewer.get();
            const { WebGLRenderTarget, FloatType } = window.orbeFixture.three;
            const hdr = new WebGLRenderTarget(size.width, size.height, { type: FloatType });
            const read = () => {
              gl.setRenderTarget(hdr);
              gl.clear();
              gl.render(scene, camera);
              gl.setRenderTarget(null);
            };
            read();
            const luminance = (ref: string) => {
              const values: number[] = [];
              scene.getObjectByName(ref).getObjectByName("body").traverse((mesh: any) => {
                if (!mesh.isMesh) return;
                const position = mesh.geometry.attributes.position;
                for (let i = 0; i < position.count; i += 1) {
                  const p = mesh.localToWorld(camera.position.clone().fromBufferAttribute(position, i)).project(camera);
                  const x = Math.round(((p.x + 1) / 2) * size.width);
                  const y = Math.round(((p.y + 1) / 2) * size.height);
                  if (x < 0 || y < 0 || x >= size.width || y >= size.height) continue;
                  const pixel = new Float32Array(4);
                  gl.readRenderTargetPixels(hdr, x, y, 1, 1, pixel);
                  values.push(0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2]);
                }
              });
              values.sort((a, b) => a - b);
              return +values[Math.floor(values.length * 0.9)].toFixed(3);
            };
            const leds = { led1: luminance("@led1"), led_l: luminance("@nano.led_l"), led_pwr: luminance("@nano.led_pwr") };
            const glowing = (material: any) =>
              material?.emissiveIntensity > 0 && (material.emissive?.r > 0 || material.emissive?.g > 0 || material.emissive?.b > 0);
            const hidden: { visible: boolean }[] = [];
            const groups = new Set<any>();
            scene.traverse((object: any) => {
              if (!object.isMesh) return;
              const materials = Array.isArray(object.material) ? object.material : [object.material];
              if (materials.some(glowing)) groups.add(object.parent);
            });
            for (const group of groups) {
              group?.traverse((node: any) => {
                if (!node.isMesh) return;
                const materials = Array.isArray(node.material) ? node.material : [node.material];
                const glow = materials.some(glowing);
                const clear = materials.some((material: any) => material?.transparent || material?.opacity < 1);
                if (glow || clear) {
                  hidden.push(node);
                  node.visible = false;
                }
              });
            }
            read();
            const pixels = new Float32Array(size.width * size.height * 4);
            gl.readRenderTargetPixels(hdr, 0, 0, size.width, size.height, pixels);
            let outside = 0;
            for (let i = 0; i < pixels.length; i += 4) {
              const y = 0.2126 * pixels[i]! + 0.7152 * pixels[i + 1]! + 0.0722 * pixels[i + 2]!;
              if (y > outside) outside = y;
            }
            for (const node of hidden) node.visible = true;
            hdr.dispose();
            return { theme, fraction, ...leds, outside: +outside.toFixed(3) };
          }, { fraction, theme });
          glow.push(row);
          expect(row.outside < 1, JSON.stringify(row)).toBe(true);
        }
      }
      const leds = ["led1", "led_l", "led_pwr"] as const;
      for (const theme of ["dark", "light"] as const) {
        const [below, at] = glow.filter((row) => row.theme === theme);
        expect(
          below !== undefined &&
            at !== undefined &&
            leds.every((led) => below[led] < 1 && Math.abs(at[led] - 1) <= 0.03),
          JSON.stringify(glow),
        ).toBe(true);
      }
    } finally {
      await context.close();
    }
  });

  /** Pixels the bloom pass, or the hover outline, changes. `hide` covers every glowing body and
   * the clear lens over it, so a bloom difference is something other than an LED blooming. The
   * read is the composited frame, because a metal highlight may pass luminance 1 and still must
   * not bloom. */
  function effectDelta(page: Page, hide: boolean, effect: "bloom" | "outline" = "bloom"): Promise<{ pixels: number; max: number; leds: Record<string, number> }> {
    return page.evaluate(async ([hide, effect]) => {
      const state = () => window.orbeViewer.get();
      const findEffect = (scene: any) => {
        let found: any;
        const walk = (node: any) => {
          if (node === undefined || node === null) return;
          const object = node.object;
          const bloom = object?.luminanceMaterial !== undefined && typeof object.intensity === "number";
          const outline = object?.visibleEdgeColor !== undefined && object?.blendMode?.opacity !== undefined;
          if (effect === "bloom" ? bloom : outline) found = object;
          for (const child of node.children ?? []) walk(child);
        };
        walk(scene.__r3f);
        if (found === undefined) throw new Error(`no ${effect} effect`);
        return found;
      };
      const frame = () =>
        new Promise<Uint8Array>((resolve, reject) => {
          const live = state();
          const subscribe = live.internal?.subscribe;
          if (typeof subscribe !== "function") {
            reject(new Error(`no frame subscribe, keys ${Object.keys(live.internal ?? {}).join(",")}`));
            return;
          }
          const ref = {
            current: () => {
              try {
                stop();
                const { gl } = state();
                const context = gl.getContext();
                const width = gl.domElement.width;
                const height = gl.domElement.height;
                context.bindFramebuffer(context.FRAMEBUFFER, null);
                const pixels = new Uint8Array(width * height * 4);
                context.readPixels(0, 0, width, height, context.RGBA, context.UNSIGNED_BYTE, pixels);
                resolve(pixels);
              } catch (error) {
                reject(error instanceof Error ? error : new Error(String(error)));
              }
            },
          };
          const stop = subscribe(ref, 2, { getState: state });
          const timer = setTimeout(() => reject(new Error(`frame did not render, subscribers ${live.internal.subscribers?.length}`)), 2000);
          const finish = ref.current;
          ref.current = () => {
            clearTimeout(timer);
            finish();
          };
        });
      const glowing = (material: any) =>
        material?.emissiveIntensity > 0 && (material.emissive?.r > 0 || material.emissive?.g > 0 || material.emissive?.b > 0);
      const hideLeds = (scene: any) => {
        const hidden: { visible: boolean }[] = [];
        const groups = new Set<any>();
        scene.traverse((object: any) => {
          if (!object.isMesh) return;
          const materials = Array.isArray(object.material) ? object.material : [object.material];
          if (materials.some(glowing) && object.parent !== undefined) groups.add(object.parent);
        });
        for (const group of groups) {
          group.traverse((node: any) => {
            if (!node.isMesh) return;
            const materials = Array.isArray(node.material) ? node.material : [node.material];
            const glow = materials.some(glowing);
            const clear = materials.some((material: any) => material?.transparent || material?.opacity < 1);
            if (glow || clear) {
              hidden.push(node);
              node.visible = false;
            }
          });
        }
        return hidden;
      };
      const { scene } = state();
      const found = findEffect(scene);
      const hidden = hide ? hideLeds(scene) : [];
      // The bloom's strength, or the outline's blend opacity, goes to 0 for the second frame.
      const strength = effect === "bloom" ? found : found.blendMode.opacity;
      const key = effect === "bloom" ? "intensity" : "value";
      const before = strength[key];
      try {
        const on = await frame();
        strength[key] = 0;
        const off = await frame();
        let energy = 0;
        let pixels = 0;
        let max = 0;
        for (let i = 0; i < on.length; i += 4) {
          energy += on[i]! + on[i + 1]! + on[i + 2]!;
          const delta = Math.max(Math.abs(on[i]! - off[i]!), Math.abs(on[i + 1]! - off[i + 1]!), Math.abs(on[i + 2]! - off[i + 2]!));
          if (delta > max) max = delta;
          if (delta > 1) pixels += 1;
        }
        if (energy === 0) throw new Error("the composited frame read back blank");
        // Each LED's peak: the largest change within 6 px of its glow body's centre on screen.
        const { camera, gl } = state();
        const width = gl.domElement.width;
        const height = gl.domElement.height;
        const leds: Record<string, number> = {};
        for (const ref of ["@led1", "@nano.led_l", "@nano.led_pwr"]) {
          const group = scene.getObjectByName(ref);
          let body: any;
          group?.traverse((node: any) => {
            const materials = Array.isArray(node.material) ? node.material : [node.material];
            if (node.isMesh && materials.some((material: any) => material?.emissive !== undefined && material.emissiveIntensity > 0)) body = node;
          });
          if (body === undefined) continue;
          body.geometry.boundingBox ?? body.geometry.computeBoundingBox();
          const centre = body.geometry.boundingBox.getCenter(body.position.clone()).applyMatrix4(body.matrixWorld).project(camera);
          const cx = Math.round(((centre.x + 1) / 2) * width);
          const cy = Math.round(((centre.y + 1) / 2) * height);
          let peak = 0;
          for (let y = Math.max(0, cy - 6); y <= Math.min(height - 1, cy + 6); y += 1) {
            for (let x = Math.max(0, cx - 6); x <= Math.min(width - 1, cx + 6); x += 1) {
              const i = (y * width + x) * 4;
              peak = Math.max(peak, Math.abs(on[i]! - off[i]!), Math.abs(on[i + 1]! - off[i + 1]!), Math.abs(on[i + 2]! - off[i + 2]!));
            }
          }
          leds[ref] = peak;
        }
        return { pixels, max, leds };
      } finally {
        strength[key] = before;
        for (const node of hidden) node.visible = true;
        await frame();
      }
    }, [hide, effect] as const);
  }

  const bloomDelta = (page: Page, hide: boolean) => effectDelta(page, hide, "bloom");
  // A slow page can still draw the last lit frame a while after the drive turns the LEDs
  // down, so wait for the bloom to clear rather than for a fixed time. It must clear.
  const unlit = (page: Page, label: string) =>
    expect
      .poll(() => bloomDelta(page, false), { message: label, timeout: 10_000 })
      .toMatchObject({ pixels: 0 });
  const outlineDelta = (page: Page) => effectDelta(page, false, "outline");

  const drive = (page: Page, seq: number, amps: number) =>
    page.evaluate(
      ([seq, amps]) => {
        const { source, script } = window.orbeFixture;
        source.push({ ...script.idle, seq, led_a: [amps, amps, amps] });
      },
      [seq, amps] as const,
    );

  async function orbit(page: Page, polarDeg: number, azimuthDeg: number) {
    await page.evaluate(({ polarDeg, azimuthDeg }) => {
      const { camera, controls } = window.orbeViewer.get();
      const target = controls.target;
      const distance = camera.position.distanceTo(target);
      const polar = (polarDeg * Math.PI) / 180;
      const azimuth = (azimuthDeg * Math.PI) / 180;
      camera.position.set(
        target.x + distance * Math.sin(polar) * Math.sin(azimuth),
        target.y + distance * Math.cos(polar),
        target.z + distance * Math.sin(polar) * Math.cos(azimuth),
      );
      camera.up.set(0, 1, 0);
      camera.lookAt(target);
      controls.update();
    }, { polarDeg, azimuthDeg });
    await still(page);
  }

  // 20 mA is twice the full current, so each LED's glow body passes the bloom threshold
  // by itself, not only through a highlight.
  const LIT_A = 0.02;
  const ORBIT_POSES = [
    { name: "23 deg off vertical at 90 deg azimuth", polar: 23, azimuth: 90 },
    { name: "30 deg off vertical at 100 deg azimuth", polar: 30, azimuth: 100 },
    { name: "straight down", polar: 0, azimuth: 90 },
  ];

  test("only a lit LED blooms, from the side, from overhead and from the top view", async ({ browser, host }) => {
    const { context, page } = await open(browser, host.base);
    try {
      let seq = 1100;
      for (const theme of ["light", "dark"] as const) {
        await setStageTheme(page, theme);
        await drive(page, seq, LIT_A);
        seq += 1;
        await page.waitForTimeout(300);
        await still(page);
        const lit = await bloomDelta(page, false);
        expect(Object.keys(lit.leds).sort(), `${theme} lit LEDs ${JSON.stringify(lit)}`).toEqual(["@led1", "@nano.led_l", "@nano.led_pwr"]);
        for (const [ref, peak] of Object.entries(lit.leds)) expect(peak > 0, `${theme} ${ref} did not bloom ${JSON.stringify(lit)}`).toBe(true);
        const outline = await outlineDelta(page);
        expect(outline.pixels, `${theme} nothing hovered, yet the outline drew ${JSON.stringify(outline)}`).toBe(0);
        for (const pose of ORBIT_POSES) {
          await orbit(page, pose.polar, pose.azimuth);
          const beside = await bloomDelta(page, true);
          expect(beside.pixels, `${theme} ${pose.name} ${JSON.stringify(beside)}`).toBe(0);
        }
        await toggleView(page, "OrthographicCamera");
        const top = await bloomDelta(page, true);
        expect(top.pixels, `${theme} top view ${JSON.stringify(top)}`).toBe(0);
        // Every LED off, with its die and lens drawn: nothing blooms.
        await drive(page, seq, 0);
        seq += 1;
        await unlit(page, `${theme} top view, every LED off`);
        await toggleView(page, "PerspectiveCamera");
        // Off, or at half the full current, a highlight on an LED's die or lens does not bloom.
        for (const amps of [0, 0.005]) {
          await drive(page, seq, amps);
          seq += 1;
          await unlit(page, `${theme} every LED at ${amps} A`);
          for (const pose of ORBIT_POSES) {
            await orbit(page, pose.polar, pose.azimuth);
            const off = await bloomDelta(page, false);
            expect(off.pixels, `${theme} ${pose.name}, every LED at ${amps} A ${JSON.stringify(off)}`).toBe(0);
          }
        }
      }
    } finally {
      await context.close();
    }
  });

  test("hovering the Nano and leaving draws no outline after, and its LEDs still bloom", async ({ browser, host }) => {
    const { context, page } = await open(browser, host.base);
    try {
      await drive(page, 1200, LIT_A);
      await page.waitForTimeout(300);
      await still(page);
      const before = await bloomDelta(page, false);
      expect(Object.keys(before.leds).sort()).toEqual(["@led1", "@nano.led_l", "@nano.led_pwr"]);

      const leave = async () => {
        await page.mouse.move(...(Object.values(await pointOnMat(page)) as [number, number]));
        await expect.poll(() => label(page)).toBeNull();
        await page.waitForTimeout(100);
      };
      const board = await hoverUntil(page, await pointAt(page, "@nano", [3, -6, 4.2]), "@nano");
      expect(board).not.toBeNull();
      const hovered = await outlineDelta(page);
      expect(hovered.pixels > 0, `the hovered Nano has no outline ${JSON.stringify(hovered)}`).toBe(true);
      await leave();
      const outline = await outlineDelta(page);
      expect(outline.pixels, `the outline stayed after the pointer left ${JSON.stringify(outline)}`).toBe(0);
      const after = await bloomDelta(page, false);
      for (const [ref, peak] of Object.entries(before.leds)) {
        expect(peak > 0, `${ref} did not bloom ${JSON.stringify(before)}`).toBe(true);
        expect(after.leds[ref], `${ref}'s bloom after a hover, before ${JSON.stringify(before)}`).toBeGreaterThanOrEqual(peak - 2);
      }

      // From an orbit pose where its metal passes luminance 1, the hovered Nano still does not bloom.
      await orbit(page, 23, 90);
      expect(await hoverUntil(page, await pointAt(page, "@nano", [3, -6, 4.2]), "@nano")).not.toBeNull();
      const metal = await bloomDelta(page, true);
      expect(metal.pixels, `with the Nano hovered, something other than an LED bloomed ${JSON.stringify(metal)}`).toBe(0);
      await leave();
    } finally {
      await context.close();
    }
  });
});
