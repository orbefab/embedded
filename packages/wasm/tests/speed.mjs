// Times every Block of a registry build in the package's worker.js, in headless Chromium, and
// fails when one is too slow to play live. Each run is one row, and the report is made from rows.
// A Worker run is the page's dedicated Worker, unthrottled. A main-thread run imports the same
// worker.js on the page under Chrome's 4x CPU throttle, which does not reach a Worker. Runs are
// interleaved: run 1 of every Block and mode, then run 2. docs/wasm.md#speed says more.
// Usage: node packages/wasm/tests/speed.mjs PKG_DIR REGISTRY_DIR [--profile BLOCK]
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { loadavg } from "node:os";
import { extname, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { chromium } from "@playwright/test";
import { loadBlock } from "../../viewer/src/registry.ts";

const RUNS = 5;
const WARM_MS = 200;
const SIM_MS = 2000;
const FIRST_PICTURE_BUDGET_MS = 5000;
const PROFILE_TOP = 15;
const PROFILE_INTERVAL_US = 100;

/** Where a run plays, how hard the CPU is throttled, and the real-time factor its median needs. */
const MODES = {
  worker: { label: "Worker 1x", where: "worker", throttle: 1, bar: 1.5 },
  main4x: { label: "main thread 4x", where: "main", throttle: 4, bar: 1.25 },
};

const args = process.argv.slice(2);
const at = args.indexOf("--profile");
const profiled = at >= 0 ? args.splice(at, 2)[1] : undefined;
const [pkgDir, registryDir] = args;
if (pkgDir === undefined || registryDir === undefined || (at >= 0 && profiled === undefined)) {
  console.error("usage: node packages/wasm/tests/speed.mjs PKG_DIR REGISTRY_DIR [--profile BLOCK]");
  process.exit(2);
}

const index = pathToFileURL(`${resolve(registryDir)}/index.json`);
const readBytes = async (url) => new Uint8Array(await readFile(url));
const ids = JSON.parse(await readFile(index, "utf8"))
  .items.filter((row) => row.type === "block")
  .map((row) => row.name)
  .filter((name, i, all) => all.indexOf(name) === i);
if (profiled !== undefined && !ids.includes(profiled)) {
  console.error(`speed: the registry has no Block ${profiled}. It has ${ids.join(", ")}`);
  process.exit(2);
}
// An ESP32-S3 Block (`devkitc-*`) is not held to the AVR bar, because it is not real time.
// AVR Blocks listed here are exempt from one or both bars until speed work fixes them:
// - `rate`: skipped from play-rate runs (Worker 1x and main thread 4x).
// - `firstPicture`: exempt from the Worker first-picture budget (5 s).
const EXEMPT = {
  "nano-buzzer": {
    rate: "runs about 10x slower than real time",
  },
  "nano-lcd1602": {
    rate: "the stock HelloWorld rewrites the LCD with no delay, tens of thousands of pin edges a second, and runs about 1.1x real time",
  },
  "nano-max7219-scroll": {
    rate: "MD_Parola scrolling over hardware SPI runs about 36 simulated ms per wall second natively, about 28x slower than real time",
  },
  "nano-neopixel-ring": {
    rate: "bit-banging 16 WS2812B pixels at 800 kHz runs about 0.88x real time in the Worker and 0.22x on the main thread at 4x throttle",
  },
  "nano-oled": {
    rate: "I2C startup runs over 10x slower than real time during warm-up",
    firstPicture: "takes about 12 s in the Worker to draw its first frame over I2C",
  },
};
const paced = ids.filter((id) => !id.startsWith("devkitc-") && EXEMPT[id]?.rate === undefined);
const blocks = new Map();
for (const id of ids) {
  const { files } = await loadBlock(index, id, readBytes);
  blocks.set(id, JSON.stringify([...files].map(([path, bytes]) => [path, Buffer.from(bytes).toString("base64")])));
}

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm" };
const ROOTS = [
  ["/pkg/", resolve(pkgDir)],
  ["/", resolve(import.meta.dirname, "speed")],
];
const server = createServer(async (request, response) => {
  const path = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
  const block = path.startsWith("/block/") ? blocks.get(path.slice("/block/".length)) : undefined;
  if (block !== undefined) {
    response.writeHead(200, { "Content-Type": "application/json" }).end(block);
    return;
  }
  const [prefix, dir] = ROOTS.find(([prefix]) => path.startsWith(prefix));
  const file = resolve(dir, path.slice(prefix.length));
  try {
    if (!file.startsWith(`${dir}/`)) throw new Error(`${path} is outside ${dir}`);
    const body = await readFile(file);
    response.writeHead(200, { "Content-Type": TYPES[extname(file)] ?? "application/octet-stream" }).end(body);
  } catch {
    response.writeHead(404).end();
  }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
const base = `http://127.0.0.1:${server.address().port}`;

let executablePath = process.env.ORBE_CHROMIUM;
if (executablePath === undefined || executablePath === "") executablePath = chromium.executablePath();
let browser;
try {
  browser = await chromium.launch({ executablePath, headless: true });
} catch (error) {
  console.error(`speed: no Chromium at ${executablePath}. This check installs no browser. Set ORBE_CHROMIUM,`);
  console.error("or install the one Playwright expects with `pnpm exec playwright install chromium`.");
  console.error(error.message);
  server.close();
  process.exit(2);
}

/** One run in a fresh page. With `profile`, the result also holds the CPU profile of the timed
 * span. */
async function measure(block, mode, profile = false) {
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  try {
    await page.goto(`${base}/index.html`);
    await page.waitForFunction(() => window.speedReady === true);
    const cdp = await page.context().newCDPSession(page);
    let recorded = null;
    if (profile) {
      await cdp.send("Profiler.enable");
      await cdp.send("Profiler.setSamplingInterval", { interval: PROFILE_INTERVAL_US });
      recorded = new Promise((done) => cdp.once("Profiler.consoleProfileFinished", (event) => done(event.profile)));
    }
    await cdp.send("Emulation.setCPUThrottlingRate", { rate: mode.throttle });
    const job = { block, where: mode.where, warmMs: WARM_MS, simMs: SIM_MS, profile };
    const out = await page.evaluate((job) => window.speed(job), job);
    if (recorded !== null) out.profile = await recorded;
    if (errors.length > 0) throw new Error(errors.join("; "));
    const expected = Math.round(out.simMs / 16);
    if (Math.abs(out.simMs - SIM_MS) > 16 || out.ticks !== expected) {
      throw new Error(`${block} ran ${out.simMs} simulated ms in ${out.ticks} ticks, expected ${SIM_MS} in ${expected}`);
    }
    if (mode.where === "worker" && typeof out.firstPictureMs !== "number") {
      throw new Error(`${block} reported no first-picture time`);
    }
    return out;
  } finally {
    await page.close();
  }
}

const load = () => loadavg().map((n) => n.toFixed(2)).join(" ");

if (profiled !== undefined) {
  const mode = MODES.main4x;
  const out = await measure(profiled, mode, true);
  const profile = out.profile;
  const nodes = new Map(profile.nodes.map((node) => [node.id, node]));
  const self = new Map();
  for (const id of profile.samples) {
    const name = (nodes.get(id).callFrame.functionName || "(anonymous)").replace(/\[[0-9a-f]{16}\]/g, "");
    self.set(name, (self.get(name) ?? 0) + 1);
  }
  const total = profile.samples.length;
  console.log(`speed: ${profiled}, ${mode.label}, ${(out.wallMs / out.simMs * 1000).toFixed(1)} ms per simulated s, ${total} samples every ${PROFILE_INTERVAL_US} us, Chromium ${browser.version()}`);
  console.log("self   function");
  for (const [name, count] of [...self].sort((a, b) => b[1] - a[1]).slice(0, PROFILE_TOP)) {
    console.log(`${((100 * count) / total).toFixed(1).padStart(5)}% ${name}`);
  }
  await browser.close();
  server.close();
  process.exit(0);
}

const aside = ids.filter((id) => !paced.includes(id));
if (aside.length > 0) console.log(`speed: not held to the bar: ${aside.join(", ")}`);
const firstPictureExempt = ids.filter((id) => EXEMPT[id]?.firstPicture !== undefined);
if (firstPictureExempt.length > 0) {
  console.log(`speed: exempt from first-picture budget: ${firstPictureExempt.map((id) => `${id} (${EXEMPT[id].firstPicture})`).join(", ")}`);
}
console.log(`speed: Chromium ${browser.version()}, ${paced.length} Blocks, ${RUNS} runs each, ${SIM_MS} simulated ms after ${WARM_MS} ms. Load ${load()}`);
const rows = [];
for (let run = 0; run < RUNS; run += 1) {
  for (const block of paced) {
    for (const [mode, how] of Object.entries(MODES)) {
      const { wallMs, simMs, firstPictureMs } = await measure(block, how);
      rows.push({ block, mode, run, msPerSimS: (wallMs / simMs) * 1000, firstPictureMs });
    }
  }
}
console.log(`speed: load after ${load()}`);
await browser.close();
server.close();

/** Wall ms per simulated s of each (Block, mode), its real-time factor at the median, and for
 * Worker runs the wall ms from start to first picture. */
const cells = paced.flatMap((block) =>
  Object.entries(MODES).map(([mode, how]) => {
    const matching = rows.filter((r) => r.block === block && r.mode === mode);
    const ms = matching.map((r) => r.msPerSimS).sort((a, b) => a - b);
    const median = ms[Math.floor(ms.length / 2)];
    let firstPicture = null;
    if (how.where === "worker") {
      const fp = matching.map((r) => r.firstPictureMs).sort((a, b) => a - b);
      firstPicture = { median: fp[Math.floor(fp.length / 2)], min: fp[0], max: fp[fp.length - 1] };
    }
    return { block, how, median, min: ms[0], max: ms[ms.length - 1], x: 1000 / median, firstPicture };
  }),
);
const width = Math.max(...paced.map((id) => id.length));
const labelWidth = Math.max(...Object.values(MODES).map((how) => how.label.length));
let failedRate = 0;
let failedFirstPicture = 0;
for (const c of cells) {
  const ratePass = c.x >= c.how.bar;
  if (!ratePass) failedRate += 1;
  let fpPass = true;
  let fpText = "";
  if (c.firstPicture !== null) {
    const exempt = EXEMPT[c.block]?.firstPicture !== undefined;
    fpPass = exempt || c.firstPicture.median <= FIRST_PICTURE_BUDGET_MS;
    if (!fpPass) failedFirstPicture += 1;
    const fpRange = `${c.firstPicture.median.toFixed(0)} ms (${c.firstPicture.min.toFixed(0)} to ${c.firstPicture.max.toFixed(0)})`;
    const fpStatus = exempt ? "exempt" : `budget ${FIRST_PICTURE_BUDGET_MS} ms${fpPass ? "" : " FAIL"}`;
    fpText = `; first picture ${fpRange}, ${fpStatus}`;
  }
  const pass = ratePass && fpPass;
  const ms = `${c.median.toFixed(1)} ms per simulated s (${c.min.toFixed(1)} to ${c.max.toFixed(1)})`;
  console.log(`${pass ? "ok  " : "FAIL"} ${c.block.padEnd(width)}  ${c.how.label.padEnd(labelWidth)}  ${ms}, ${c.x.toFixed(2)}x real time, bar ${c.how.bar}x${fpText}`);
}
const failures = [];
if (failedRate > 0) failures.push(`${failedRate} of ${cells.length} medians miss their bar`);
if (failedFirstPicture > 0) failures.push(`${failedFirstPicture} of ${paced.length} first-picture medians exceed ${FIRST_PICTURE_BUDGET_MS} ms`);
console.log(failures.length === 0 ? "speed: every Block keeps up" : `speed: ${failures.join("; ")}`);
process.exit(failures.length === 0 ? 0 : 1);
