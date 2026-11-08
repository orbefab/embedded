// Drives the built page in `?block=` mode in Chromium: nano-button-led loaded from a registry
// build and run in orbe-wasm's Worker, then play, hold @btn1 until @led1 lights, release until
// it goes dark. A Block the registry lacks ends the session with why. The page is served by
// `vite preview`, which serves the registry at /registry/ and the package at /orbe-wasm/ from
// .scratch/wasm, where scripts/check-block.sh builds them. Usage: node block.mjs [--renders <dir>]
import { existsSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { chromium } from "playwright-core";
import { preview } from "vite";

const APP = resolve(import.meta.dirname, "..");
const SCRATCH = resolve(APP, "../../.scratch/wasm");
const rendersAt = process.argv.indexOf("--renders");
const renders = rendersAt >= 0 ? process.argv[rendersAt + 1] : undefined;
if (renders !== undefined) mkdirSync(renders, { recursive: true });

const executable = process.env.ORBE_CHROMIUM ?? chromium.executablePath();
if (!existsSync(executable)) {
  console.error(`No Chromium at ${executable}.`);
  console.error("This script installs no browser. Set ORBE_CHROMIUM to a Chromium or Chrome binary,");
  console.error("or install the one playwright-core 1.64.1 expects with `pnpm --filter @orbe/viewer-page exec playwright-core install chromium`.");
  process.exit(2);
}
for (const [path, how] of [
  [resolve(APP, "dist/index.html"), "pnpm --filter @orbe/viewer-page build"],
  [resolve(SCRATCH, "pkg/worker.js"), "scripts/wasm.sh .scratch/wasm/pkg"],
  [resolve(SCRATCH, "registry/index.json"), "orbe registry build . .scratch/wasm/registry"],
]) {
  if (!existsSync(path)) {
    console.error(`No ${path}. Run \`${how}\` first, or scripts/check-block.sh.`);
    process.exit(2);
  }
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
const glowAbove = (page, above, timeout = 10_000) =>
  page.waitForFunction(
    (above) => {
      const glow = window.orbeViewer.get().scene.getObjectByName("@led1").getObjectByName("body").material.emissiveIntensity;
      return above ? glow > 0.5 : glow < 0.05;
    },
    above,
    { timeout }
  );
const shown = (page, id) => page.evaluate((id) => document.querySelector(`[data-testid="${id}"]`)?.textContent ?? null, id);
const stateIs = (page, state) =>
  page.waitForFunction((state) => document.querySelector('[data-testid="state"]')?.textContent === state, state, { timeout: 10_000 });

async function open(query) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
  const logs = [];
  page.on("console", (message) => logs.push(message.text()));
  page.on("pageerror", (error) => logs.push(`pageerror: ${error.message}`));
  await page.goto(`${base}?${query}`);
  return { page, logs };
}

try {
  const { page, logs } = await open("block=nano-button-led");
  await page.waitForFunction(() => window.orbeViewer?.get().scene.getObjectByName("@btn1") !== undefined, null, { timeout: 30_000 });
  await stateIs(page, "paused");
  await page.waitForTimeout(800);
  check("the page draws nano-button-led from the registry, paused at t = 0", (await shown(page, "time")) === "t = 0.000000 s", await shown(page, "time"));
  check(
    "@led1's and @btn1's models load from the registry's files",
    await page.evaluate(() => {
      const { scene } = window.orbeViewer.get();
      return scene.getObjectByName("@led1")?.getObjectByName("body") !== undefined && scene.getObjectByName("@btn1")?.getObjectByName("stem") !== undefined;
    }),
    await shown(page, "errors")
  );
  check("the fixture's test hook is absent on a Block's page", await page.evaluate(() => window.orbeFixture === undefined), null);
  await shot(page, "block-opened");

  await page.click('[data-testid="play"]');
  await stateIs(page, "playing");
  await page.waitForTimeout(500);
  const time = Number((await shown(page, "time"))?.match(/t = ([0-9.]+) s/)?.[1]);
  check("play runs the Worker's session past t = 0", time > 0, await shown(page, "time"));

  const stem = await pointOn(page, "@btn1", "stem");
  await page.mouse.move(stem.x, stem.y);
  await page.mouse.down();
  await glowAbove(page, true);
  check("held, @led1 glows past 0.5", true, null);
  await shot(page, "block-pressed-lit");

  await page.mouse.up();
  await glowAbove(page, false);
  check("released, @led1 goes dark", true, null);
  await shot(page, "block-released");
  check("the session refused no command the page sent, and every model loaded", (await shown(page, "errors")) === null, await shown(page, "errors"));
  check("the page logged no error", !logs.some((line) => line.startsWith("pageerror")), logs);
  await page.close();
} catch (error) {
  check("a step threw", false, String(error?.message ?? error).split("\n")[0]);
}

try {
  const { page, logs } = await open("block=no-such-block");
  await page.waitForSelector('[data-testid="closed"]', { timeout: 10_000 });
  check(
    "a Block the registry lacks ends the session and says why",
    (await shown(page, "closed")) === "the session ended: the Block did not load: the registry has no no-such-block",
    await shown(page, "closed")
  );
  check("the page logged no error", !logs.some((line) => line.startsWith("pageerror")), logs);
  await page.close();
} catch (error) {
  check("a step threw", false, String(error?.message ?? error).split("\n")[0]);
}

await browser.close();
await server.close();
console.log(failed === 0 ? "all block checks passed" : `${failed} block checks failed`);
process.exit(failed === 0 ? 0 : 1);
