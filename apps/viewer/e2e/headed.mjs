// Drives a live headed session of the first target in Chromium: `orbe open --headed
// --no-browser`, then play, hold @btn1 until @led1 lights, and check `orbe get @led1` reads what
// the viewer shows. The session runs in its own runtime folder, and ends however the run does.
// Usage: node headed.mjs <orbe binary> [--renders <dir>]
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { chromium } from "playwright-core";

const REPO = resolve(import.meta.dirname, "../../..");
const orbeBinary = resolve(process.argv[2] ?? "");
const rendersAt = process.argv.indexOf("--renders");
const renders = rendersAt >= 0 ? process.argv[rendersAt + 1] : undefined;
if (renders !== undefined) mkdirSync(renders, { recursive: true });
if (!existsSync(orbeBinary)) {
  console.error(`No orbe at ${orbeBinary}. Usage: node headed.mjs <orbe binary> [--renders <dir>]`);
  process.exit(2);
}
const executable = process.env.ORBE_CHROMIUM ?? chromium.executablePath();
if (!existsSync(executable)) {
  console.error(`No Chromium at ${executable}.`);
  console.error("This script installs no browser. Set ORBE_CHROMIUM to a Chromium or Chrome binary,");
  console.error("or install the one playwright-core 1.64.1 expects with `pnpm --filter @orbe/viewer-page exec playwright-core install chromium`.");
  process.exit(2);
}

/** led-5mm's `readings.brightness.full`, the current the viewer draws at glow 1. */
const LED_FULL_A = 0.01;

const runtime = mkdtempSync(join(tmpdir(), "orbe-headed-"));
const env = { ...process.env, XDG_RUNTIME_DIR: runtime };
delete env.ORBE_SESSION;
delete env.ORBE_VIEWER_DIR;
/** `orbe <args> --json` in the repo's root, as its envelope. */
const orbe = (...args) => {
  try {
    return JSON.parse(execFileSync(orbeBinary, [...args, "--json"], { cwd: REPO, env, encoding: "utf8" }));
  } catch (error) {
    if (typeof error.stdout === "string" && error.stdout.startsWith("{")) return JSON.parse(error.stdout);
    throw error;
  }
};

let failed = 0;
function check(name, pass, observed) {
  if (!pass) failed += 1;
  console.log(`${pass ? "ok  " : "FAIL"} ${name}`);
  if (!pass || process.env.ORBE_E2E_VERBOSE) console.log(`     observed: ${JSON.stringify(observed)}`);
}
const shot = async (page, name) => {
  if (renders !== undefined) await page.screenshot({ path: `${renders}/${name}.png` });
};

/** The client pixel of the centre of the mesh `node` under the instance `ref`. */
const pointOn = (page, ref, node) =>
  page.evaluate(
    ([ref, node]) => {
      const { scene, camera, size } = window.orbeViewer.get();
      const object = scene.getObjectByName(ref)?.getObjectByName(node);
      if (object === undefined) throw new Error(`no ${ref} ${node}`);
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
/** @led1's glow as the viewer draws it: its mean current over full. */
const glow = (page) =>
  page.evaluate(() => window.orbeViewer.get().scene.getObjectByName("@led1").getObjectByName("body").material.emissiveIntensity);
const shown = (page, id) => page.evaluate((id) => document.querySelector(`[data-testid="${id}"]`)?.textContent ?? null, id);
const stateIs = (page, state) =>
  page.waitForFunction((state) => document.querySelector('[data-testid="state"]')?.textContent === state, state, { timeout: 10_000 });

let open = false;
const browser = await chromium.launch({
  executablePath: executable,
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
});
try {
  const opened = orbe("open", "blocks/nano-button-led/scene.jsonc", "--headed", "--no-browser");
  open = opened.ok || opened.state !== "closed";
  const url = opened.data?.url;
  check("`orbe open --headed --no-browser` opens the scene and gives the page's URL", opened.ok && /^http:\/\/127\.0\.0\.1:\d+\/#t=[0-9a-f]{64}$/.test(url ?? ""), {
    ok: opened.ok,
    error: opened.error,
    url: url?.replace(/#t=.*/, "#t=<token>"),
  });
  if (!opened.ok) throw new Error("no session to watch");

  const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
  const logs = [];
  page.on("console", (message) => logs.push(message.text()));
  page.on("pageerror", (error) => logs.push(`pageerror: ${error.message}`));
  await page.goto(url);
  await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 30_000 });
  await stateIs(page, "paused");
  await page.waitForTimeout(800);
  check("the page draws the session's scene, paused at t = 0", (await shown(page, "time")) === "t = 0.000000 s", await shown(page, "time"));
  check("the fixture's test hook is absent on a live page", await page.evaluate(() => window.orbeFixture === undefined), null);
  await shot(page, "headed-opened");

  await page.click('[data-testid="play"]');
  await stateIs(page, "playing");
  await page.waitForTimeout(500);
  const before = orbe("get", "@led1");
  check("play runs the session: `orbe get` sees it playing past t = 0", before.state === "playing" && before.t_ps > 0, {
    state: before.state,
    t_ps: before.t_ps,
  });
  await shot(page, "headed-playing");

  const stem = await pointOn(page, "@btn1", "stem");
  await page.mouse.move(stem.x, stem.y);
  await page.mouse.down();
  await page.waitForFunction(
    () => window.orbeViewer.get().scene.getObjectByName("@led1").getObjectByName("body").material.emissiveIntensity > 0.5,
    null,
    { timeout: 10_000 }
  );
  await page.waitForTimeout(300);
  await shot(page, "headed-pressed-lit");

  const paused = orbe("pause");
  await stateIs(page, "paused");
  await page.waitForTimeout(200);
  const lit = orbe("get", "@led1");
  const drawn = await glow(page);
  const read = lit.data.value.i / LED_FULL_A;
  check(
    "held and paused, `orbe get @led1` reads on at the current the viewer glows at",
    paused.ok && lit.data.value.on === true && Math.abs(drawn - read) <= 0.05 * read,
    { t_ps: lit.t_ps, get_i: lit.data.value.i, get_on: lit.data.value.on, viewer_glow: drawn, get_glow: read }
  );
  await shot(page, "headed-paused-lit");

  await page.mouse.up();
  await page.click('[data-testid="play"]');
  await stateIs(page, "playing");
  await page.waitForFunction(
    () => window.orbeViewer.get().scene.getObjectByName("@led1").getObjectByName("body").material.emissiveIntensity < 0.05,
    null,
    { timeout: 10_000 }
  );
  const off = orbe("get", "@led1");
  check("released and playing, @led1 goes dark and `orbe get` reads it off", off.data.value.on === false, {
    get_on: off.data.value.on,
    viewer_glow: await glow(page),
  });
  await shot(page, "headed-released");
  check("the session refused no command the page sent", (await shown(page, "errors")) === null, await shown(page, "errors"));
  check("the page logged no error", !logs.some((line) => line.startsWith("pageerror")), logs);

  const closed = orbe("close");
  open = false;
  await page.waitForSelector('[data-testid="closed"]', { timeout: 10_000 });
  check(
    "`orbe close` ends the page's session and the page says why",
    closed.ok && (await shown(page, "closed")) === "the session ended: `orbe close` ended the session",
    await shown(page, "closed")
  );
  await shot(page, "headed-closed");
} catch (error) {
  check("a step threw", false, String(error?.message ?? error).split("\n")[0]);
} finally {
  await browser.close();
  if (open) orbe("close");
  rmSync(runtime, { recursive: true, force: true });
}

console.log(failed === 0 ? "all headed checks passed" : `${failed} headed checks failed`);
process.exit(failed === 0 ? 0 : 1);
