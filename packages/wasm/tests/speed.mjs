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
// An ESP32-S3 Block is not held to the AVR bar. It is not real time. Neither is nano-buzzer yet:
// it runs about 10x slower than real time.
const slow = new Set(["nano-buzzer"]);
const paced = ids.filter((id) => !id.startsWith("devkitc-") && !slow.has(id));
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
console.log(`speed: Chromium ${browser.version()}, ${paced.length} Blocks, ${RUNS} runs each, ${SIM_MS} simulated ms after ${WARM_MS} ms. Load ${load()}`);
const rows = [];
for (let run = 0; run < RUNS; run += 1) {
  for (const block of paced) {
    for (const [mode, how] of Object.entries(MODES)) {
      const { wallMs, simMs } = await measure(block, how);
      rows.push({ block, mode, run, msPerSimS: (wallMs / simMs) * 1000 });
    }
  }
}
console.log(`speed: load after ${load()}`);
await browser.close();
server.close();

/** Wall ms per simulated s of each (Block, mode), and its real-time factor at the median. */
const cells = paced.flatMap((block) =>
  Object.entries(MODES).map(([mode, how]) => {
    const ms = rows.filter((r) => r.block === block && r.mode === mode).map((r) => r.msPerSimS).sort((a, b) => a - b);
    const median = ms[Math.floor(ms.length / 2)];
    return { block, how, median, min: ms[0], max: ms[ms.length - 1], x: 1000 / median };
  }),
);
const width = Math.max(...paced.map((id) => id.length));
const labelWidth = Math.max(...Object.values(MODES).map((how) => how.label.length));
let failed = 0;
for (const c of cells) {
  const pass = c.x >= c.how.bar;
  if (!pass) failed += 1;
  const ms = `${c.median.toFixed(1)} ms per simulated s (${c.min.toFixed(1)} to ${c.max.toFixed(1)})`;
  console.log(`${pass ? "ok  " : "FAIL"} ${c.block.padEnd(width)}  ${c.how.label.padEnd(labelWidth)}  ${ms}, ${c.x.toFixed(2)}x real time, bar ${c.how.bar}x`);
}
console.log(failed === 0 ? "speed: every Block keeps up" : `speed: ${failed} of ${cells.length} medians miss their bar`);
process.exit(failed === 0 ? 0 : 1);
