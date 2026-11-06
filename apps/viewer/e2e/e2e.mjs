// Drives the built fixture page in Chromium and checks what a person can do with it: presses,
// releases, the top view, hover and the glow. Usage: node e2e.mjs [--renders <dir>]
import { existsSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { chromium } from "playwright-core";
import { preview } from "vite";

const APP = resolve(import.meta.dirname, "..");
const rendersAt = process.argv.indexOf("--renders");
const renders = rendersAt >= 0 ? process.argv[rendersAt + 1] : undefined;
if (renders !== undefined) mkdirSync(renders, { recursive: true });

const executable = process.env.ORBE_CHROMIUM ?? chromium.executablePath();
if (!existsSync(executable)) {
  console.error(`No Chromium at ${executable}.`);
  console.error("This script installs no browser. Set ORBE_CHROMIUM to a Chromium or Chrome binary,");
  console.error("or install the one playwright-core 1.64.1 expects with `npx playwright-core install chromium`.");
  process.exit(2);
}
if (!existsSync(resolve(APP, "dist/index.html"))) {
  console.error(`No build at ${resolve(APP, "dist")}. Run \`npm run build --workspace apps/viewer\` first.`);
  process.exit(2);
}

const server = await preview({ root: APP, logLevel: "silent", preview: { host: "127.0.0.1", port: 0 } });
const base = server.resolvedUrls?.local[0];
const browser = await chromium.launch({
  executablePath: executable,
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
});

let failed = 0;
function check(name, pass, observed) {
  if (!pass) failed += 1;
  console.log(`${pass ? "ok  " : "FAIL"} ${name}`);
  if (!pass || process.env.ORBE_E2E_VERBOSE) console.log(`     observed: ${JSON.stringify(observed)}`);
}
/** Runs one case, so a case that throws fails and the rest still run. */
async function guarded(run) {
  try {
    await run();
  } catch (error) {
    check("a case threw", false, String(error?.message ?? error).split("\n")[0]);
  }
}
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const near = (a, b, tolerance = 1e-3) => a.length === b.length && a.every((value, i) => Math.abs(value - b[i]) <= tolerance);

async function open({ frame = "idle", touch = false } = {}) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 720 }, hasTouch: touch });
  const page = await context.newPage();
  const logs = [];
  page.on("console", (message) => logs.push(message.text()));
  page.on("pageerror", (error) => logs.push(`pageerror: ${error.message}`));
  await page.goto(`${base}?frame=${frame}`);
  await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, {
    timeout: 30_000,
  });
  await settle(page);
  const commands = () => logs.filter((line) => /^(press|release) @\S+$/.test(line));
  return { context, page, logs, commands, cdp: await context.newCDPSession(page) };
}

const settle = (page, ms = 800) => page.waitForTimeout(ms);

/** Waits until the controls' damping has stopped moving the camera: 5 drawn frames in a row
 * in which the target and the zoom each move under 0.01. The damping steps once per drawn
 * frame, so frames, not wall time, say when it is done. Throws after 600 frames. */
async function still(page) {
  const result = await page.evaluate(
    () =>
      new Promise((done) => {
        let last;
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
      })
  );
  if (result.calm < 5) throw new Error(`the camera still moved after ${result.frames} frames: ${JSON.stringify(result.now)}`);
}

/** Presses `T` and waits until the `type` camera is the controls' and stands still. */
async function toggleView(page, type) {
  await page.keyboard.press("t");
  await page.waitForFunction((type) => {
    const { camera, controls } = window.orbeViewer.get();
    return camera.type === type && controls?.object === camera;
  }, type);
  await still(page);
}

/** Records the id of each pointer that goes down on the canvas in `window.lastPointer`. */
const watchPointer = (page) =>
  page.evaluate(() => {
    window.orbeViewer.get().gl.domElement.addEventListener("pointerdown", (event) => {
      window.lastPointer = event.pointerId;
    });
  });

/** Dispatches a `type` pointer event for the last pointer on the canvas, and says whether
 * the canvas had that pointer's capture before and after. */
const dispatchPointer = (page, type, init) =>
  page.evaluate(
    ([type, init]) => {
      const canvas = window.orbeViewer.get().gl.domElement;
      const before = canvas.hasPointerCapture(window.lastPointer);
      canvas.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: window.lastPointer, ...init }));
      return { before, after: canvas.hasPointerCapture(window.lastPointer) };
    },
    [type, init]
  );

/** The canvas's pointerup, pointercancel and lostpointercapture listeners, counted by type. */
async function endListeners(cdp) {
  const { result } = await cdp.send("Runtime.evaluate", { expression: "window.orbeViewer.get().gl.domElement" });
  const { listeners } = await cdp.send("DOMDebugger.getEventListeners", { objectId: result.objectId });
  const by = {};
  for (const { type } of listeners) if (["pointerup", "pointercancel", "lostpointercapture"].includes(type)) by[type] = (by[type] ?? 0) + 1;
  return by;
}

const errorsShown = (page) => page.evaluate(() => document.querySelector('[data-testid="errors"]')?.textContent ?? null);

/** The client pixel of the centre of the mesh `node` under the instance `ref`, or of the
 * instance itself with no `node`. */
const pointOn = (page, ref, node) =>
  page.evaluate(
    ([ref, node]) => {
      const { scene, camera, size } = window.orbeViewer.get();
      const group = scene.getObjectByName(ref);
      const object = node === undefined ? group : group?.getObjectByName(node);
      if (object === undefined) throw new Error(`no ${ref} ${node ?? ""}`);
      let box;
      object.traverse((mesh) => {
        if (!mesh.isMesh) return;
        mesh.geometry.computeBoundingBox();
        const own = mesh.geometry.boundingBox.clone().applyMatrix4(mesh.matrixWorld);
        box = box === undefined ? own : box.union(own);
      });
      const at = box.getCenter(box.min.clone()).project(camera);
      return { x: ((at.x + 1) / 2) * size.width, y: ((1 - at.y) / 2) * size.height };
    },
    [ref, node]
  );

/** The client pixel of a point in the part frame of the instance `ref`. */
const pointAt = (page, ref, at) =>
  page.evaluate(
    ([ref, at]) => {
      const { scene, camera, size } = window.orbeViewer.get();
      const group = scene.getObjectByName(ref);
      const screen = group.localToWorld(group.position.clone().set(...at)).project(camera);
      return { x: ((screen.x + 1) / 2) * size.width, y: ((1 - screen.y) / 2) * size.height };
    },
    [ref, at]
  );

/** A client pixel over the empty mat, right of every part. */
const pointOnMat = (page) =>
  page.evaluate(() => {
    const { scene, camera, size } = window.orbeViewer.get();
    let mat;
    scene.traverse((object) => {
      if (object.isMesh && object.geometry.type === "PlaneGeometry") mat = object;
    });
    const root = scene.getObjectByName("@nano").parent;
    const parts = root.children.map((group) => {
      const box = group.getWorldPosition(group.position.clone());
      return box;
    });
    const right = Math.max(...parts.map((p) => p.x)) + 25;
    const at = mat.getWorldPosition(mat.position.clone());
    at.x = right;
    const screen = at.project(camera);
    return { x: ((screen.x + 1) / 2) * size.width, y: ((1 - screen.y) / 2) * size.height };
  });

/** What the pointer at a client pixel hits first: an instance or lead ref, `mat`, or null. */
const hitAt = (page, { x, y }) =>
  page.evaluate(
    ([x, y]) => {
      const { scene, camera, size, raycaster, pointer } = window.orbeViewer.get();
      raycaster.setFromCamera(pointer.clone().set((x / size.width) * 2 - 1, 1 - (y / size.height) * 2), camera);
      const hit = raycaster.intersectObjects(scene.children, true)[0];
      if (hit === undefined) return null;
      if (hit.object.geometry.type === "PlaneGeometry") return "mat";
      for (let at = hit.object; at !== null; at = at.parent) if (at.userData.orbe) return at.userData.orbe.ref;
      return hit.object.type;
    },
    [x, y]
  );

const label = (page) =>
  page
    .locator('[data-testid="hover-label"]')
    .innerText({ timeout: 300 })
    .catch(() => null);

/** Moves the mouse near `at` until the hover label starts with `ref`. */
async function hoverUntil(page, at, ref) {
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

async function click(page, at) {
  await page.mouse.move(at.x, at.y);
  await page.mouse.down();
  await page.waitForTimeout(80);
  await page.mouse.up();
  await page.waitForTimeout(80);
}

const touch = (cdp, type, points) =>
  cdp.send("Input.dispatchTouchEvent", { type, touchPoints: points.map(({ x, y, id }) => ({ x, y, id })) });

/** Points the camera at the mesh `node` under `ref` from `distance` mm. */
const lookAt = (page, ref, node, distance) =>
  page.evaluate(
    ([ref, node, distance]) => {
      const { scene, camera, controls } = window.orbeViewer.get();
      const target = scene.getObjectByName(ref).getObjectByName(node).getWorldPosition(camera.position.clone());
      const offset = camera.position.clone().sub(controls.target).setLength(distance);
      controls.target.copy(target);
      camera.position.copy(target).add(offset);
      controls.update();
    },
    [ref, node, distance]
  );

const pushScene = (page, edit) =>
  page.evaluate((source) => {
    const { source: fixture, firstTarget } = window.orbeFixture;
    fixture.push(new Function("scene", source)(firstTarget));
  }, edit);

try {
  await guarded(async () => {
    const { context, page, commands, cdp } = await open();
    const stem = await pointOn(page, "@btn1", "stem");
    await page.mouse.move(stem.x, stem.y);
    await page.mouse.down();
    await page.waitForTimeout(80);
    const afterDown = commands();
    const held = await endListeners(cdp);
    await page.mouse.up();
    await page.waitForTimeout(80);
    check(
      "mouse down on btn1 presses it, and up releases it",
      same(afterDown, ["press @btn1"]) && same(commands(), ["press @btn1", "release @btn1"]),
      { afterDown, afterUp: commands() }
    );
    const released = await endListeners(cdp);
    check(
      "a press adds one pointerup, pointercancel and lostpointercapture listener to the canvas, and its release removes them",
      same(held, { pointerup: 1, pointercancel: 1, lostpointercapture: 1 }) && same(released, {}),
      { held, released }
    );
    const touchAction = await page.evaluate(() => getComputedStyle(document.querySelector('[data-testid="viewer"]')).touchAction);
    check("the viewer box's touch-action is none, so a finger on it never scrolls the page", touchAction === "none", { touchAction });
    await context.close();
  });

  await guarded(async () => {
    const { context, page, commands } = await open();
    const stem = await pointOn(page, "@btn1", "stem");
    await page.mouse.move(stem.x, stem.y);
    await page.mouse.down({ button: "right" });
    await page.waitForTimeout(80);
    await page.mouse.up({ button: "right" });
    await page.waitForTimeout(80);
    check("the right button alone on btn1 sends nothing", same(commands(), []), { commands: commands() });
    await context.close();
  });

  await guarded(async () => {
    const { context, page, commands } = await open();
    const stem = await pointOn(page, "@btn1", "stem");
    await watchPointer(page);
    await page.mouse.move(stem.x, stem.y);
    await page.mouse.down();
    await page.waitForTimeout(80);
    await page.mouse.down({ button: "right" });
    await page.mouse.up({ button: "right" });
    await page.waitForTimeout(80);
    const afterChord = commands();
    const rightUp = await dispatchPointer(page, "pointerup", { pointerType: "mouse", button: 2, buttons: 1 });
    await page.waitForTimeout(80);
    const afterRightUp = commands();
    await page.mouse.up();
    await page.waitForTimeout(80);
    check(
      "with the left button held on btn1, the right button going down and up does not release it, and the left up does",
      same(afterChord, ["press @btn1"]) && same(afterRightUp, ["press @btn1"]) && same(commands(), ["press @btn1", "release @btn1"]),
      { afterChord, rightUp, afterRightUp, afterLeftUp: commands() }
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page, commands } = await open();
    const stem = await pointOn(page, "@btn1", "stem");
    await page.evaluate(() => {
      const { source } = window.orbeFixture;
      window.realSend = source.send;
      source.send = async (command) =>
        command.cmd === "press"
          ? { ok: false, error: { code: "usage", message: "no part @btn1", hint: "" } }
          : Promise.reject(new Error("the session closed"));
    });
    await click(page, stem);
    const shown = await errorsShown(page);
    await page.evaluate(() => {
      window.orbeFixture.source.send = window.realSend;
    });
    await click(page, stem);
    check(
      "a press the session refuses and a release whose send rejects are both listed, and the next press works",
      shown === "press @btn1: no part @btn1\nrelease @btn1: the session closed" && same(commands(), ["press @btn1", "release @btn1"]),
      { shown, commands: commands() }
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page, commands, cdp } = await open({ touch: true });
    const stem = await pointOn(page, "@btn1", "stem");
    await touch(cdp, "touchStart", [{ ...stem, id: 1 }]);
    await page.waitForTimeout(150);
    const afterStart = commands();
    await touch(cdp, "touchCancel", []);
    await page.waitForTimeout(200);
    check(
      "a touch on btn1 that the browser cancels releases it",
      same(afterStart, ["press @btn1"]) && same(commands(), ["press @btn1", "release @btn1"]),
      { afterStart, afterCancel: commands() }
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page, commands, cdp } = await open({ touch: true });
    const stem = await pointOn(page, "@btn1", "stem");
    await watchPointer(page);
    await touch(cdp, "touchStart", [{ ...stem, id: 1 }]);
    await page.waitForTimeout(150);
    const capture = await dispatchPointer(page, "pointercancel", { pointerType: "touch" });
    await page.waitForTimeout(150);
    const afterCancel = commands();
    await touch(cdp, "touchEnd", []);
    await page.waitForTimeout(150);
    check(
      "a pointercancel on btn1's pointer releases it while the canvas still has the capture, and the lift sends nothing",
      capture.before && capture.after && same(afterCancel, ["press @btn1", "release @btn1"]) && same(commands(), afterCancel),
      { capture, afterCancel, final: commands() }
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page, commands } = await open();
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
    check(
      "losing the pointer capture releases btn1 before the up, and the up on the mat sends nothing",
      lost && same(afterLost, ["press @btn1", "release @btn1"]) && same(commands(), afterLost),
      { canvasHadCapture: lost, afterLost, final: commands(), upOn: await hitAt(page, mat) }
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page, commands, cdp } = await open({ touch: true });
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
    check(
      "a second finger on btn1 while it is held sends no second press, and lifting it does not release the first finger's",
      secondFingerOn === "@btn1" && same(afterSecondLifts, ["press @btn1"]) && same(commands(), ["press @btn1", "release @btn1"]),
      { secondFingerOn, afterSecondLifts, afterFirstLifts: commands() }
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page, commands } = await open();
    const stem = await pointOn(page, "@btn1", "stem");
    await page.mouse.move(stem.x, stem.y);
    await page.mouse.down();
    await page.waitForTimeout(80);
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await page.waitForTimeout(80);
    const afterBlur = commands();
    await page.mouse.up();
    await page.waitForTimeout(80);
    check(
      "the window losing focus releases btn1 once",
      same(afterBlur, ["press @btn1", "release @btn1"]) && same(commands(), afterBlur),
      { afterBlur, afterUp: commands() }
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page, commands } = await open();
    const stem = await pointOn(page, "@btn1", "stem");
    await page.mouse.move(stem.x, stem.y);
    await page.mouse.down();
    await page.waitForTimeout(80);
    await pushScene(
      page,
      `return { ...scene, session: "reopened",
        instances: scene.instances.filter((i) => i.name !== "btn1"),
        wires: scene.wires.filter((w) => !w.ends.some((e) => e.startsWith("btn1."))),
        channels: { ...scene.channels, switches: ["@nano.sw_reset"] } };`
    );
    await page.waitForFunction(() => window.orbeViewer.get().scene.getObjectByName("@btn1") === undefined);
    await settle(page);
    await page.mouse.up();
    await page.waitForTimeout(100);
    const afterUp = commands();
    const reset = await hoverUntil(page, await pointOn(page, "@nano.sw_reset", "button"), "@nano.sw_reset");
    if (reset !== null) await click(page, reset);
    check(
      "a press held across a new scene sends no release, and the next press works",
      same(afterUp, ["press @btn1"]) &&
        same(commands(), ["press @btn1", "press @nano.sw_reset", "release @nano.sw_reset"]),
      { afterUp, final: commands() }
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page, commands } = await open();
    const board = await hoverUntil(page, await pointAt(page, "@nano", [3, -6, 4.2]), "@nano");
    if (board !== null) await click(page, board);
    const mat = await pointOnMat(page);
    const matHit = await hitAt(page, mat);
    await click(page, mat);
    check("clicks on the Nano's board and on the mat send nothing", board !== null && matHit === "mat" && same(commands(), []), {
      board,
      matHit,
      commands: commands(),
    });
    const reset = await hoverUntil(page, await pointOn(page, "@nano.sw_reset", "button"), "@nano.sw_reset");
    if (reset !== null) await click(page, reset);
    check(
      "a click on the Nano's reset button presses and releases @nano.sw_reset",
      same(commands(), ["press @nano.sw_reset", "release @nano.sw_reset"]),
      { reset, commands: commands() }
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page } = await open();
    await toggleView(page, "OrthographicCamera");
    const live = await page.evaluate(() => {
      const viewer = window.orbeViewer;
      return { keys: Object.keys(viewer), camera: viewer.get().camera.type, stale: "camera" in viewer && viewer.camera !== viewer.get().camera };
    });
    check("window.orbeViewer gives the live state", same(live.keys, ["get"]) && live.camera === "OrthographicCamera" && !live.stale, live);

    const axes = await page.evaluate(() => {
      const { camera } = window.orbeViewer.get();
      const column = (i) => camera.matrixWorld.elements.slice(i * 4, i * 4 + 3).map((v) => +v.toFixed(4));
      return { up: camera.up.toArray(), right: column(0), screenUp: column(1) };
    });
    check(
      "the top view's screen-right is world +X and its screen-up world -Z, with camera.up -Z",
      near(axes.up, [0, 0, -1]) && near(axes.right, [1, 0, 0]) && near(axes.screenUp, [0, 0, -1]),
      axes
    );
    if (renders !== undefined) await page.screenshot({ path: `${renders}/top.png` });

    const state = () =>
      page.evaluate(() => {
        const { controls, camera } = window.orbeViewer.get();
        return { target: controls.target.toArray(), zoom: camera.zoom };
      });
    const mat = await pointOnMat(page);
    const drag = async (dx, dy) => {
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
    const afterUp = up.to;
    await page.mouse.wheel(0, -400);
    await still(page);
    const afterWheel = await state();
    check(
      "in the top view a left drag pans: dragging left moves the target +X, dragging up moves it +Z",
      left.moved[0] > 5 && Math.abs(left.moved[2]) < 0.5 && up.moved[2] > 5 && Math.abs(up.moved[0]) < 0.5,
      { left, up }
    );
    check("in the top view the wheel zooms", afterWheel.zoom > afterUp.zoom * 1.05, { before: afterUp.zoom, after: afterWheel.zoom });
    await context.close();
  });

  await guarded(async () => {
    const { context, page, cdp } = await open({ touch: true });
    await toggleView(page, "OrthographicCamera");
    const mat = await pointOnMat(page);
    const target = () => page.evaluate(() => window.orbeViewer.get().controls.target.toArray());
    const before = await target();
    await touch(cdp, "touchStart", [{ ...mat, id: 1 }]);
    for (let step = 1; step <= 10; step += 1) {
      await touch(cdp, "touchMove", [{ x: mat.x - step * 20, y: mat.y, id: 1 }]);
      await page.waitForTimeout(16);
    }
    await touch(cdp, "touchEnd", []);
    await still(page);
    const after = await target();
    check("in the top view a one-finger drag pans", after[0] - before[0] > 5, { before, after });
    await context.close();
  });

  await guarded(async () => {
    const { context, page } = await open();
    await lookAt(page, "@nano.j2", "pin-10", 45);
    await settle(page, 1200);
    const pin = await hoverUntil(page, await pointOn(page, "@nano.j2", "pin-10"), "@nano.A6");
    const text = await label(page);
    check(
      "hover on a not-simulated pin shows its reason",
      pin !== null && text === "@nano.A6\nnot simulated: The engine's atmega328p kind has the DIP's pins, with no ADC6 or ADC7 pad.",
      { pin, text }
    );
    if (renders !== undefined) await page.screenshot({ path: `${renders}/hover-a6.png` });
    await lookAt(page, "@nano.led_tx", "body", 45);
    await settle(page, 1200);
    const led = await hoverUntil(page, await pointOn(page, "@nano.led_tx", "body"), "@nano.led_tx");
    const ledText = await label(page);
    check(
      "hover on a not-simulated part shows its reason",
      led !== null && ledText === "@nano.led_tx\nnot simulated: The CH340G drives it, and its model covers only TXD and DTR.",
      { led, text: ledText }
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page, logs } = await open();
    const drawn = () =>
      page.evaluate(() => ({
        canvas: document.querySelectorAll("canvas").length,
        r1: window.orbeViewer.get().scene.getObjectByName("@r1") !== undefined,
        errors: document.querySelector('[data-testid="errors"]')?.textContent ?? null,
      }));
    await pushScene(
      page,
      `return { ...scene, session: "shunt",
        instances: scene.instances.map((i) => (i.name === "r1" ? { ...i, params: { resistance: 0.01 } } : i)) };`
    );
    await settle(page);
    const shunt = { ...(await drawn()), warnings: logs.filter((line) => line.includes("no 4-band code")).length };
    check(
      "a 0.01 ohm resistor keeps its model's bands, warns once, and the scene still draws",
      shunt.canvas === 1 && shunt.r1 && shunt.errors === null && shunt.warnings === 1,
      shunt
    );
    await pushScene(
      page,
      `const led = scene.parts["led-5mm"];
      return { ...scene, session: "bad-led",
        parts: { ...scene.parts, "led-5mm": { ...led, view: { ...led.view, glow: [{ node: "lens", by: "brightness" }] } } } };`
    );
    await settle(page);
    const bad = await drawn();
    check(
      "a part whose view names a missing node is listed as an error, and the scene still draws",
      bad.canvas === 1 && bad.r1 && bad.errors === '@led1: led-5mm: the GLB has no node "lens"',
      bad
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page } = await open();
    const colours = await page.evaluate(() => {
      const { scene } = window.orbeViewer.get();
      const material = (ref) => scene.getObjectByName(ref).getObjectByName("body").material;
      const ratio = (colour) => [colour.g / colour.r, colour.b / colour.r].map((v) => +v.toFixed(3));
      return {
        led_tx: `#${material("@nano.led_tx").color.getHexString()}`,
        led_rx: `#${material("@nano.led_rx").color.getHexString()}`,
        led_pwr: ratio(material("@nano.led_pwr").emissive),
        red: ratio(material("@nano.led_rx").color),
      };
    });
    check(
      "each 0603 LED takes its colour's tint, and the lit power LED glows in the red tint",
      colours.led_tx === "#c5d6b0" && colours.led_rx === "#e0b0a4" && near(colours.led_pwr, colours.red),
      colours
    );
    await context.close();
  });

  await guarded(async () => {
    const { context, page } = await open();
    const glow = [];
    for (const fraction of [0.95, 1.0]) {
      await page.evaluate((fraction) => {
        const { source, script } = window.orbeFixture;
        const amps = 0.01 * fraction;
        source.push({ ...script.idle, seq: 1000 + fraction, led_a: [amps, amps, amps] });
      }, fraction);
      await settle(page, 300);
      glow.push(
        await page.evaluate((fraction) => {
          const { scene, gl, camera, size } = window.orbeViewer.get();
          const { WebGLRenderTarget, FloatType } = window.orbeFixture.three;
          const hdr = new WebGLRenderTarget(size.width, size.height, { type: FloatType });
          gl.setRenderTarget(hdr);
          gl.clear();
          gl.render(scene, camera);
          gl.setRenderTarget(null);
          const luminance = (ref) => {
            const values = [];
            scene.getObjectByName(ref).getObjectByName("body").traverse((mesh) => {
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
          const row = { fraction, led1: luminance("@led1"), led_l: luminance("@nano.led_l"), led_pwr: luminance("@nano.led_pwr") };
          hdr.dispose();
          return row;
        }, fraction)
      );
    }
    const [below, at] = glow;
    const leds = ["led1", "led_l", "led_pwr"];
    check(
      "each LED's lit luminance is under 1 at 0.95 of its full current and within 3 % of 1 at full",
      leds.every((led) => below[led] < 1 && Math.abs(at[led] - 1) <= 0.03),
      glow
    );
    await context.close();
  });

  if (renders !== undefined) {
    for (const frame of ["idle", "pressed"]) {
      const { context, page } = await open({ frame });
      await settle(page, 1200);
      await page.screenshot({ path: `${renders}/${frame}.png` });
      await lookAt(page, "@nano.led_pwr", "body", 30);
      await settle(page, 1200);
      await page.screenshot({ path: `${renders}/${frame}-nano-leds.png` });
      await context.close();
    }
  }
} finally {
  await browser.close();
  await new Promise((done) => server.httpServer.close(done));
}

console.log(failed === 0 ? "all e2e checks passed" : `${failed} e2e checks failed`);
process.exit(failed === 0 ? 0 : 1);
