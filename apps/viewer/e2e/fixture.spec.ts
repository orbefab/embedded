import type { Browser, BrowserContext, CDPSession, Page } from "@playwright/test";
import { collectLogs, expect, pointOn, rendersDir, shot, test, textOf, waitState } from "../../../e2e/support";

const VIEW = { width: 1280, height: 720 };
const near = (a: number[], b: number[], tolerance = 1e-3) => a.length === b.length && a.every((value, i) => Math.abs(value - b[i]) <= tolerance);
const SWITCH: Record<string, string> = { "@btn1": "stem", "@nano.sw_reset": "button" };

/** The page with no token and no `?block=`, which runs nano-button-led in the Worker. */
async function open(browser: Browser, base: string, { touch = false } = {}) {
  const context = await browser.newContext({ viewport: VIEW, hasTouch: touch });
  const page = await context.newPage();
  const logs = collectLogs(page, "all");
  await page.goto(`${base}/`);
  await page.waitForFunction(
    () => {
      const scene = window.orbeViewer?.get().scene;
      return scene?.getObjectByName("@bb") !== undefined && scene.getObjectByName("@btn1")?.getObjectByName("stem") !== undefined;
    },
    null,
    { timeout: 120_000 },
  );
  await waitState(page, "paused");
  await still(page);
  const rest = await pose(page, "@btn1");
  const cdp = await context.newCDPSession(page);
  return { context, page, logs, rest, cdp };
}

/** Damping steps once per `controls.update()`, which the rig calls once per drawn frame.
 * The checks read the settled pose, so step `update()` here until the pose is still. */
async function still(page: Page) {
  const result = await page.evaluate(() => {
    let last: number[] | undefined;
    let calm = 0;
    let frames = 0;
    let now: number[] = [];
    while (calm < 5 && frames < 600) {
      window.orbeViewer.get().controls.update();
      const { controls, camera } = window.orbeViewer.get();
      now = [...controls.target.toArray(), ...camera.position.toArray(), camera.zoom];
      calm = last !== undefined && now.every((value, i) => Math.abs(value - last[i]) < 0.01) ? calm + 1 : 0;
      last = now;
      frames += 1;
    }
    return { frames, calm, now };
  });
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

function pose(page: Page, ref: string): Promise<number[]> {
  const node = SWITCH[ref];
  if (node === undefined) throw new Error(`no switch node for ${ref}`);
  return page.evaluate(
    ([ref, node]) => {
      const object = window.orbeViewer.get().scene.getObjectByName(ref)?.getObjectByName(node);
      if (object === undefined) throw new Error(`no ${ref} ${node}`);
      // World millimetres. The node's own position is in the GLB's metres.
      return object.getWorldPosition(object.position.clone()).toArray() as number[];
    },
    [ref, node] as const,
  );
}

const shifted = (rest: number[], now: number[]) => now.some((value, i) => Math.abs(value - (rest[i] ?? 0)) > 0.05);

/** A press lands at `now`, and a frame shows a contact only once time has moved past it. */
async function play(page: Page): Promise<void> {
  if ((await textOf(page, "state")) === "playing") return;
  await page.click('[data-testid="play"]');
  await waitState(page, "playing");
}

/** Wait until the switch has travelled, or returned, because the Worker answers after the click. */
async function waitHeld(page: Page, ref: string, rest: number[], held: boolean): Promise<void> {
  const node = SWITCH[ref];
  await page.waitForFunction(
    ({ ref, node, rest, held }) => {
      const object = window.orbeViewer.get().scene.getObjectByName(ref)?.getObjectByName(node);
      if (object === undefined) return false;
      const now = object.getWorldPosition(object.position.clone()).toArray() as number[];
      const moved = now.some((value: number, i: number) => Math.abs(value - rest[i]) > 0.05);
      return moved === held;
    },
    { ref, node, rest, held },
    { timeout: 10_000 },
  );
}

/** The switch is still where `held` says, after long enough for a late frame to arrive. */
async function remains(page: Page, ref: string, rest: number[], held: boolean): Promise<void> {
  await page.waitForTimeout(400);
  const now = await pose(page, ref);
  expect(shifted(rest, now), `${ref} held=${held} ${JSON.stringify({ rest, now })}`).toBe(held);
}

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

/** A point in the frame's margin. The scene fills the view inside that margin, so a drag
 * that starts here is on the mat and the camera pans. */
const pointOnMat = (page: Page) =>
  page.evaluate(() => {
    const { scene, camera, size, raycaster, pointer } = window.orbeViewer.get();
    const kind = (x: number, y: number) => {
      raycaster.setFromCamera(pointer.clone().set((x / size.width) * 2 - 1, 1 - (y / size.height) * 2), camera);
      const hit = raycaster.intersectObjects(scene.children, true)[0];
      return hit?.object.geometry?.type === "PlaneGeometry";
    };
    const x = size.width - 28;
    for (const y of [size.height / 2, 28, size.height - 28]) {
      if (kind(x, y)) return { x, y };
    }
    throw new Error("the right margin does not show the mat");
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

async function downOn(page: Page, ref: string, rest: number[]) {
  await play(page);
  const node = SWITCH[ref];
  if (node === undefined) throw new Error(`no switch node for ${ref}`);
  const at = await pointOn(page, ref, node);
  await page.mouse.move(at.x, at.y);
  await page.mouse.down();
  await waitHeld(page, ref, rest, true);
  return at;
}

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

const LEDS = ["@led1", "@nano.led_l", "@nano.led_pwr", "@nano.led_tx", "@nano.led_rx"] as const;

function intensities(page: Page): Promise<Record<string, number>> {
  return page.evaluate((refs) => {
    const scene = window.orbeViewer.get().scene;
    const out: Record<string, number> = {};
    for (const ref of refs) {
      const material = scene.getObjectByName(ref)?.getObjectByName("body")?.material;
      const emissive = material?.emissive;
      const lit = emissive?.r > 0 || emissive?.g > 0 || emissive?.b > 0;
      if (lit && typeof material?.emissiveIntensity === "number") out[ref] = material.emissiveIntensity;
    }
    return out;
  }, LEDS);
}

/** Pixels the bloom pass, or the hover outline, changes. `hide` true covers every glowing body
 * and the clear lens over it, so a bloom difference is something other than an LED blooming.
 * `"full"` covers only a body at intensity 1 or more, so a dimmer LED is still drawn. The
 * read is the composited frame, because a metal highlight may pass luminance 1 and still must
 * not bloom. */
function effectDelta(page: Page, hide: boolean | "full", effect: "bloom" | "outline" = "bloom"): Promise<{ pixels: number; max: number; leds: Record<string, number> }> {
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
    const glowing = (material: any, minimum: number) => {
      const intensity = material?.emissiveIntensity ?? 0;
      const lit = minimum === 0 ? intensity > 0 : intensity >= minimum;
      return lit && (material?.emissive?.r > 0 || material?.emissive?.g > 0 || material?.emissive?.b > 0);
    };
    const hideLeds = (scene: any, minimum: number) => {
      const hidden: { visible: boolean }[] = [];
      const groups = new Set<any>();
      scene.traverse((object: any) => {
        if (!object.isMesh) return;
        const materials = Array.isArray(object.material) ? object.material : [object.material];
        if (materials.some((material: any) => glowing(material, minimum)) && object.parent !== undefined) groups.add(object.parent);
      });
      for (const group of groups) {
        group.traverse((node: any) => {
          if (!node.isMesh) return;
          const materials = Array.isArray(node.material) ? node.material : [node.material];
          const glow = materials.some((material: any) => glowing(material, minimum));
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
    const hidden = hide === false ? [] : hideLeds(scene, hide === "full" ? 1 : 0);
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
    }
  }, [hide, effect] as const);
}

const bloomDelta = (page: Page, hide: boolean | "full") => effectDelta(page, hide, "bloom");
const outlineDelta = (page: Page) => effectDelta(page, false, "outline");

/** The session is paused and damping is off, so two frames differ only by the effect.
 * A click on the page would move the pointer, so the play control is clicked in the page. */
async function withStill<T>(page: Page, read: () => Promise<T>): Promise<T> {
  const playing = (await textOf(page, "state")) === "playing";
  if (playing) {
    await page.evaluate(() => document.querySelector<HTMLButtonElement>('[data-testid="play"]')?.click());
    await waitState(page, "paused");
  }
  await page.evaluate(() => {
    const controls = window.orbeViewer.get().controls;
    controls.enableDamping = false;
    controls.update();
  });
  try {
    return await read();
  } finally {
    await page.evaluate(() => {
      window.orbeViewer.get().controls.enableDamping = true;
    });
    if (playing) {
      await page.evaluate(() => document.querySelector<HTMLButtonElement>('[data-testid="play"]')?.click());
      await waitState(page, "playing");
    }
  }
}

function expectNoPixels(page: Page, read: () => Promise<{ pixels: number }>, message: string): Promise<void> {
  return withStill(page, () => expect.poll(read, { message, timeout: 10_000 }).toMatchObject({ pixels: 0 }));
}

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

const ORBIT_POSES = [
  { name: "23 deg off vertical at 90 deg azimuth", polar: 23, azimuth: 90 },
  { name: "30 deg off vertical at 100 deg azimuth", polar: 30, azimuth: 100 },
  { name: "straight down", polar: 0, azimuth: 90 },
];

/** The pointer is up and over the mat, so nothing is hovered. A press captures the pointer,
 * and until it is released a move still hovers the part that was pressed. */
async function restPointer(page: Page) {
  await page.mouse.up();
  const mat = await pointOnMat(page);
  await page.mouse.move(mat.x, mat.y);
  await expect.poll(() => label(page)).toBeNull();
}

/** An LED at intensity 1 or more blooms. One below that does not, which is read with the
 * brighter bodies hidden, because their halo reaches a dimmer LED's window. */
async function expectBloomFollowsCurrent(page: Page, label: string) {
  return withStill(page, async () => {
    const glow = await intensities(page);
    const lit = await bloomDelta(page, false);
    for (const [ref, intensity] of Object.entries(glow)) {
      if (intensity < 1) continue;
      expect(lit.leds[ref] ?? 0, `${label} ${ref} at ${intensity} ${JSON.stringify(lit)}`).toBeGreaterThan(0);
    }
    expect(Object.values(glow).some((intensity) => intensity >= 1), `${label} no LED at full ${JSON.stringify(glow)}`).toBe(true);
    await expect.poll(() => bloomDelta(page, "full"), {
      message: `${label} an LED below its full current bloomed`,
      timeout: 10_000,
    }).toMatchObject({ pixels: 0 });
    return lit;
  });
}

async function holdButton(page: Page, rest: number[]) {
  await downOn(page, "@btn1", rest);
  await page.waitForFunction(
    () => (window.orbeViewer.get().scene.getObjectByName("@led1")?.getObjectByName("body")?.material?.emissiveIntensity ?? 0) >= 1,
    null,
    { timeout: 15_000 },
  );
}

// Parallel, so CI can shard single tests. A describe that shares one page stays in default
// mode, and that whole describe stays on one shard.
test.describe.configure({ mode: "parallel" });

test.describe("viewer page", () => {
  test.describe.configure({ mode: "parallel" });

  test.describe("press and release", () => {
    test.describe.configure({ mode: "default" });
    let context: BrowserContext;
    let afterDown = false;
    let afterUp = true;
    let held: Record<string, number> = {};
    let released: Record<string, number> = {};
    let touchAction = "";

    test.beforeAll(async ({ browser, host }) => {
      const opened = await open(browser, host.base);
      context = opened.context;
      const { page, rest, cdp } = opened;
      await downOn(page, "@btn1", rest);
      afterDown = shifted(rest, await pose(page, "@btn1"));
      held = await endListeners(cdp);
      await page.mouse.up();
      await waitHeld(page, "@btn1", rest, false);
      afterUp = shifted(rest, await pose(page, "@btn1"));
      released = await endListeners(cdp);
      touchAction = await page.evaluate(() => getComputedStyle(document.querySelector('[data-testid="viewer"]')!).touchAction);
      expect(await errorsShown(page)).toBeNull();
    });

    test.afterAll(async () => {
      await context?.close();
    });

    test("mouse down on btn1 presses it, and up releases it", () => {
      expect({ afterDown, afterUp }).toEqual({ afterDown: true, afterUp: false });
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
    const { context, page, rest } = await open(browser, host.base);
    try {
      await play(page);
      const stem = await pointOn(page, "@btn1", "stem");
      await page.mouse.move(stem.x, stem.y);
      await page.mouse.down({ button: "right" });
      await page.mouse.up({ button: "right" });
      await remains(page, "@btn1", rest, false);
      expect(await textOf(page, "state")).toBe("playing");
      expect(await errorsShown(page)).toBeNull();
    } finally {
      await context.close();
    }
  });

  test("with the left button held on btn1, the right button going down and up does not release it, and the left up does", async ({ browser, host }) => {
    const { context, page, rest } = await open(browser, host.base);
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await watchPointer(page);
      await downOn(page, "@btn1", rest);
      await page.mouse.down({ button: "right" });
      await page.mouse.up({ button: "right" });
      await remains(page, "@btn1", rest, true);
      await dispatchPointer(page, "pointerup", { pointerType: "mouse", button: 2, buttons: 1 });
      await remains(page, "@btn1", rest, true);
      await page.mouse.up();
      await waitHeld(page, "@btn1", rest, false);
    } finally {
      await context.close();
    }
  });

  test("in a chord on btn1, the left button lifting first releases it while the right is still down", async ({ browser, host }) => {
    const { context, page, rest } = await open(browser, host.base);
    try {
      const stem = await pointOn(page, "@btn1", "stem");
      await page.mouse.move(stem.x, stem.y);
      await downOn(page, "@btn1", rest);
      await page.mouse.down({ button: "right" });
      await page.mouse.up();
      await waitHeld(page, "@btn1", rest, false);
      await page.mouse.up({ button: "right" });
      await remains(page, "@btn1", rest, false);
    } finally {
      await context.close();
    }
  });

  test("a touch on btn1 that the browser cancels releases it", async ({ browser, host }) => {
    const { context, page, rest, cdp } = await open(browser, host.base, { touch: true });
    try {
      await play(page);
      const stem = await pointOn(page, "@btn1", "stem");
      await touch(cdp, "touchStart", [{ ...stem, id: 1 }]);
      await waitHeld(page, "@btn1", rest, true);
      await touch(cdp, "touchCancel", []);
      await waitHeld(page, "@btn1", rest, false);
    } finally {
      await context.close();
    }
  });

  test("a finger moving while it holds btn1 keeps the press, and its lift releases it", async ({ browser, host }) => {
    const { context, page, rest, cdp } = await open(browser, host.base, { touch: true });
    try {
      await play(page);
      const stem = await pointOn(page, "@btn1", "stem");
      await touch(cdp, "touchStart", [{ ...stem, id: 1 }]);
      await waitHeld(page, "@btn1", rest, true);
      for (let step = 1; step <= 5; step += 1) {
        await touch(cdp, "touchMove", [{ x: stem.x + 3 * step, y: stem.y + 2 * step, id: 1 }]);
        await page.waitForTimeout(40);
      }
      await remains(page, "@btn1", rest, true);
      await touch(cdp, "touchEnd", []);
      await waitHeld(page, "@btn1", rest, false);
    } finally {
      await context.close();
    }
  });

  test("a pointercancel on btn1's pointer releases it while the canvas still has the capture, and the lift sends nothing", async ({ browser, host }) => {
    const { context, page, rest, cdp } = await open(browser, host.base, { touch: true });
    try {
      await play(page);
      const stem = await pointOn(page, "@btn1", "stem");
      await watchPointer(page);
      await touch(cdp, "touchStart", [{ ...stem, id: 1 }]);
      await waitHeld(page, "@btn1", rest, true);
      const capture = await dispatchPointer(page, "pointercancel", { pointerType: "touch" });
      await waitHeld(page, "@btn1", rest, false);
      await touch(cdp, "touchEnd", []);
      await remains(page, "@btn1", rest, false);
      expect(capture.before && capture.after).toBe(true);
    } finally {
      await context.close();
    }
  });

  test("losing the pointer capture releases btn1 before the up, and the up on the mat sends nothing", async ({ browser, host }) => {
    const { context, page, rest } = await open(browser, host.base);
    try {
      await play(page);
      const stem = await pointOn(page, "@btn1", "stem");
      const mat = await pointOnMat(page);
      await watchPointer(page);
      await page.mouse.move(stem.x, stem.y);
      await page.mouse.down();
      await page.mouse.move(stem.x + 1, stem.y);
      await waitHeld(page, "@btn1", rest, true);
      const lost = await page.evaluate(() => {
        const canvas = window.orbeViewer.get().gl.domElement;
        const had = canvas.hasPointerCapture(window.lastPointer);
        canvas.releasePointerCapture(window.lastPointer);
        return had;
      });
      await page.mouse.move(mat.x, mat.y, { steps: 5 });
      await waitHeld(page, "@btn1", rest, false);
      await page.mouse.up();
      await remains(page, "@btn1", rest, false);
      expect({ lost, upOn: await hitAt(page, mat) }).toEqual({ lost: true, upOn: "mat" });
    } finally {
      await context.close();
    }
  });

  test("a second finger on btn1 while it is held sends no second press, and lifting it does not release the first finger's", async ({ browser, host }) => {
    const { context, page, rest, cdp } = await open(browser, host.base, { touch: true });
    try {
      await play(page);
      const stem = await pointOn(page, "@btn1", "stem");
      const secondFingerOn = await hitAt(page, { x: stem.x + 1, y: stem.y });
      const f1 = { ...stem, id: 1 };
      const f2 = { x: stem.x + 1, y: stem.y, id: 2 };
      await touch(cdp, "touchStart", [f1]);
      await waitHeld(page, "@btn1", rest, true);
      await touch(cdp, "touchStart", [f1, f2]);
      await page.waitForTimeout(120);
      await touch(cdp, "touchEnd", [f2]);
      await remains(page, "@btn1", rest, true);
      await touch(cdp, "touchEnd", [f1]);
      await waitHeld(page, "@btn1", rest, false);
      expect(secondFingerOn).toBe("@btn1");
      expect(await errorsShown(page)).toBeNull();
    } finally {
      await context.close();
    }
  });

  test("the window losing focus releases btn1 once", async ({ browser, host }) => {
    const { context, page, rest } = await open(browser, host.base);
    try {
      await downOn(page, "@btn1", rest);
      await page.evaluate(() => window.dispatchEvent(new Event("blur")));
      await waitHeld(page, "@btn1", rest, false);
      await page.mouse.up();
      await remains(page, "@btn1", rest, false);
    } finally {
      await context.close();
    }
  });

  test.describe("clicks that send nothing, and reset", () => {
    test.describe.configure({ mode: "default" });
    let context: BrowserContext;
    let board: { x: number; y: number } | null = null;
    let matHit: string | null = null;
    let btnMoved = true;
    let resetHeld = false;
    let resetReleased = true;

    test.beforeAll(async ({ browser, host }) => {
      const opened = await open(browser, host.base);
      context = opened.context;
      const { page, rest } = opened;
      await play(page);
      board = await hoverUntil(page, await pointAt(page, "@nano", [3, -6, 4.2]), "@nano");
      if (board !== null) await click(page, board);
      const mat = await pointOnMat(page);
      matHit = await hitAt(page, mat);
      await click(page, mat);
      await remains(page, "@btn1", rest, false);
      btnMoved = shifted(rest, await pose(page, "@btn1"));
      const resetRest = await pose(page, "@nano.sw_reset");
      const reset = await hoverUntil(page, await pointOn(page, "@nano.sw_reset", "button"), "@nano.sw_reset");
      if (reset === null) throw new Error("the Nano's reset button is not under the pointer");
      await page.mouse.move(reset.x, reset.y);
      await page.mouse.down();
      await waitHeld(page, "@nano.sw_reset", resetRest, true);
      resetHeld = true;
      await page.mouse.up();
      await waitHeld(page, "@nano.sw_reset", resetRest, false);
      resetReleased = shifted(resetRest, await pose(page, "@nano.sw_reset"));
      expect(await errorsShown(page)).toBeNull();
      expect(await textOf(page, "state")).toBe("playing");
    });

    test.afterAll(async () => {
      await context?.close();
    });

    test("clicks on the Nano's board and on the mat send nothing", () => {
      expect({ board: board !== null, matHit, btnMoved }).toEqual({ board: true, matHit: "mat", btnMoved: false });
    });

    test("a click on the Nano's reset button presses and releases @nano.sw_reset", () => {
      expect({ resetHeld, resetReleased }).toEqual({ resetHeld: true, resetReleased: false });
    });
  });

  test.describe("top view", () => {
    test.describe.configure({ mode: "default" });
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
        return {
          keys: Object.keys(viewer),
          camera: viewer.get().camera.type,
          stale: "camera" in viewer && viewer.camera !== viewer.get().camera,
          drive: "orbeFixture" in window,
        };
      });
      expect(live).toEqual({ keys: ["get"], camera: "OrthographicCamera", stale: false, drive: false });
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
    test.describe.configure({ mode: "default" });
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
      await lookAt(page, "@header", "housing-1", 45);
      await page.waitForTimeout(300);
      const pin = await hoverUntil(page, await pointOn(page, "@header", "housing-1"), "@header.1");
      const text = await label(page);
      expect({ pin: pin !== null, text }).toEqual({
        pin: true,
        text: "@header.1\nnot simulated: Nothing in the circuit is wired to it.",
      });
      await shot(page, "hover-header");
    });

    test("hover on a not-simulated part shows its reason", async () => {
      await lookAt(page, "@nano.led_tx", "body", 45);
      await page.waitForTimeout(300);
      const led = await hoverUntil(page, await pointOn(page, "@nano.led_tx", "body"), "@nano.led_tx");
      const text = await label(page);
      expect({ led: led !== null, text }).toEqual({
        led: true,
        text: "@nano.led_tx\nnot simulated: The CH340G drives it, and its model covers only TXD and DTR.",
      });
    });
  });

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

  for (const theme of ["light", "dark"] as const) {
    test(`only an LED at or above its full current blooms, from the side, from overhead and from the top view, ${theme}`, async ({ browser, host }) => {
      const { context, page, rest } = await open(browser, host.base);
      try {
        await setStageTheme(page, theme);
        await restPointer(page);
        await expectNoPixels(page, () => outlineDelta(page), `${theme} nothing hovered, yet the outline drew`);
        await holdButton(page, rest);
        await still(page);
        const lit = await expectBloomFollowsCurrent(page, theme);
        expect(Object.keys(lit.leds).sort(), `${theme} lit LEDs ${JSON.stringify(lit)}`).toEqual(["@led1", "@nano.led_l", "@nano.led_pwr"]);
        for (const pose of ORBIT_POSES) {
          await orbit(page, pose.polar, pose.azimuth);
          await expectNoPixels(page, () => bloomDelta(page, true), `${theme} ${pose.name}`);
        }
        await toggleView(page, "OrthographicCamera");
        await expectNoPixels(page, () => bloomDelta(page, true), `${theme} top view`);
        await page.mouse.up();
        await waitHeld(page, "@btn1", rest, false);
        await page.waitForFunction(
          () => (window.orbeViewer.get().scene.getObjectByName("@led1")?.getObjectByName("body")?.material?.emissiveIntensity ?? 1) < 1,
          null,
          { timeout: 15_000 },
        );
        const dark = await intensities(page);
        expect(Object.values(dark).every((intensity) => intensity < 1), `${theme} ${JSON.stringify(dark)}`).toBe(true);
        await expectNoPixels(page, () => bloomDelta(page, false), `${theme} top view, the button released`);
        await toggleView(page, "PerspectiveCamera");
        for (const pose of ORBIT_POSES) {
          await orbit(page, pose.polar, pose.azimuth);
          await expectNoPixels(page, () => bloomDelta(page, false), `${theme} ${pose.name}, button released`);
        }
      } finally {
        await context.close();
      }
    });
  }

  test("hovering the Nano and leaving draws no outline after, and its lit LED still blooms", async ({ browser, host }) => {
    const { context, page, rest } = await open(browser, host.base);
    try {
      await setStageTheme(page, "dark");
      await holdButton(page, rest);
      await still(page);
      const before = await expectBloomFollowsCurrent(page, "before hover");

      const board = await hoverUntil(page, await pointAt(page, "@nano", [3, -6, 4.2]), "@nano");
      expect(board).not.toBeNull();
      const hovered = await outlineDelta(page);
      expect(hovered.pixels > 0, `the hovered Nano has no outline ${JSON.stringify(hovered)}`).toBe(true);
      await still(page);
      const after = await bloomDelta(page, false);
      const led = before.leds["@led1"] ?? 0;
      expect(led, `the button's LED did not bloom ${JSON.stringify(before)}`).toBeGreaterThan(0);
      // The pointer travels from the button to the Nano, so the camera yaws and the peak changes.
      expect(after.leds["@led1"], `@led1's bloom after a hover ${JSON.stringify(after)}`).toBeGreaterThan(0);

      // Releasing first: a move with the button down orbits the camera, so the pose would not stick.
      await page.mouse.up();
      await orbit(page, 23, 90);
      expect(await hoverUntil(page, await pointAt(page, "@nano", [3, -6, 4.2]), "@nano")).not.toBeNull();
      await still(page);
      const metal = await bloomDelta(page, true);
      // A hovered edge shimmers by a few hundred pixels. Bloom and a stuck outline are far larger.
      expect(metal.pixels, `with the Nano hovered, something other than an LED bloomed ${JSON.stringify(metal)}`).toBeLessThan(5_000);
      await restPointer(page);
      await still(page);
      const outline = await outlineDelta(page);
      expect(outline.pixels, `the outline stayed after the pointer left ${JSON.stringify(outline)}`).toBeLessThan(5_000);
    } finally {
      await context.close();
    }
  });

  if (rendersDir() !== undefined) {
    test("saves the opened and pressed renders", async ({ browser, host }) => {
      const { context, page, rest } = await open(browser, host.base);
      try {
        await shot(page, "opened");
        await holdButton(page, rest);
        await page.waitForTimeout(300);
        await shot(page, "pressed");
        await lookAt(page, "@nano.led_pwr", "body", 30);
        await page.waitForTimeout(300);
        await shot(page, "pressed-nano-leds");
      } finally {
        await context.close();
      }
    });
  }
});
