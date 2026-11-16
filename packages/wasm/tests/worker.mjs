// Runs each Block of a registry build in the package's worker.js, in a node worker thread, as a
// page runs it: loaded and checked by the viewer's registry client, opened, played, pressed. Then
// it times one simulated second of nano-button-led in a worker thread, ticking as fast as it can.
// Usage: node packages/wasm/tests/worker.mjs PKG_DIR REGISTRY_DIR
import { readdir, readFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { Worker } from "node:worker_threads";
import { loadBlock } from "../../viewer/src/registry.ts";

const [pkgDir, registryDir] = process.argv.slice(2);
if (pkgDir === undefined || registryDir === undefined) {
  console.error("usage: node packages/wasm/tests/worker.mjs PKG_DIR REGISTRY_DIR");
  process.exit(2);
}
const pkg = pathToFileURL(`${resolve(pkgDir)}/`);
const index = pathToFileURL(`${resolve(registryDir)}/index.json`);
const readBytes = async (url) => new Uint8Array(await readFile(url));

let failed = 0;
function check(name, pass, observed) {
  if (!pass) failed += 1;
  console.log(`${pass ? "ok  " : "FAIL"} ${name}`);
  if (!pass) console.log(`     observed: ${JSON.stringify(observed)}`);
}

/** A Block open in worker.js: its messages so far, and a way to wait for one. */
function open(block) {
  const worker = new Worker(new URL("worker.js", pkg));
  const seen = [];
  const waiting = new Set();
  worker.on("message", (text) => {
    const message = JSON.parse(text);
    seen.push(message);
    for (const wait of waiting) wait(message);
  });
  worker.on("error", (error) => {
    seen.push({ type: "closed", reason: `worker error: ${error.message}` });
  });
  worker.postMessage({ type: "open", name: block.name, files: [...block.files] });
  let next = 1;
  const until = (test, ms, what) =>
    new Promise((done, fail) => {
      const found = seen.find(test);
      if (found !== undefined) return done(found);
      const timer = setTimeout(() => {
        waiting.delete(wait);
        fail(new Error(`no ${what} within ${ms} ms`));
      }, ms);
      const wait = (message) => {
        if (!test(message)) return;
        clearTimeout(timer);
        waiting.delete(wait);
        done(message);
      };
      waiting.add(wait);
    });
  return {
    seen,
    until,
    async send(command) {
      const id = next++;
      worker.postMessage(JSON.stringify({ ...command, id }));
      return (await until((m) => m.type === "reply" && m.id === id, 5000, `reply to ${command.cmd}`)).reply;
    },
    frames: (from) => seen.slice(from).filter((m) => m.type === "frame"),
    close: () => worker.terminate(),
  };
}

const at = (scene, kind, ref) => scene.channels[kind].indexOf(ref);

async function buttonLed(id, board, extraLed) {
  const session = open(await loadBlock(index, id, readBytes));
  try {
    const scene = await session.until((m) => m.type === "scene", 10_000, "scene");
    const d13 = at(scene, "pins", `@${board}.D13`);
    const led = at(scene, "leds", "@led1");
    const extra = extraLed === undefined ? -1 : at(scene, "leds", extraLed);
    check(`${id}: the scene names D13, @led1 and @btn1`, d13 >= 0 && led >= 0 && at(scene, "switches", "@btn1") >= 0, scene.channels);
    const first = await session.until((m) => m.type === "frame", 5000, "first frame");
    check(`${id}: the first frame is t = 0, paused`, first.t_ps === 0 && first.state === "paused", first);

    const played = await session.send({ cmd: "play" });
    check(`${id}: play is answered playing`, played.ok && played.state === "playing", played);
    const dark = await session.until((m) => m.type === "frame" && m.t_ps >= 100e9, 10_000, "frame at 100 ms");
    check(`${id}: playing to 100 ms, D13 is low and @led1 dark`, dark.pins[d13] === 0 && dark.led_a[led] < 1e-4 && dark.speed === 1, dark);

    const mark = session.seen.length;
    const pressed = await session.send({ cmd: "press", ref: "@btn1" });
    check(`${id}: press @btn1 is answered`, pressed.ok, pressed);
    const lit = await session.until(
      (m) => m.type === "frame" && session.seen.indexOf(m) >= mark && m.pins[d13] === 1 && m.led_a[led] > 5e-3,
      5000,
      "frame with D13 high and @led1 lit"
    );
    check(`${id}: held, D13 goes high and @led1's current rises past 5 mA`, true, lit);
    if (extra >= 0) check(`${id}: ${extraLed} lights with it`, lit.led_a[extra] > 1e-3, lit.led_a);

    const released = await session.send({ cmd: "release", ref: "@btn1" });
    const after = session.seen.length;
    const off = await session.until(
      (m) => m.type === "frame" && session.seen.indexOf(m) >= after && m.pins[d13] === 0 && m.led_a[led] < 1e-4,
      5000,
      "frame with @led1 dark again"
    );
    check(`${id}: released, D13 goes low and @led1 dark`, released.ok && off.switches[at(scene, "switches", "@btn1")] === 0, off);

    const paused = await session.send({ cmd: "pause" });
    check(`${id}: pause is answered paused`, paused.ok && paused.state === "paused", paused);
    const refused = await session.send({ cmd: "step", ref: "1ms" });
    check(`${id}: a command no viewer sends is refused as usage`, !refused.ok && refused.error.code === "usage", refused);
  } catch (error) {
    check(`${id}: ${error.message}`, false, session.seen.slice(-3));
  } finally {
    await session.close();
  }
}

async function fade() {
  const id = "nano-led-fade";
  const session = open(await loadBlock(index, id, readBytes));
  try {
    const scene = await session.until((m) => m.type === "scene", 10_000, "scene");
    const d9 = at(scene, "pins", "@nano.D9");
    const led = at(scene, "leds", "@led1");
    check(`${id}: the scene names D9 and @led1`, d9 >= 0 && led >= 0, scene.channels);
    await session.send({ cmd: "play" });
    await session.until((m) => m.type === "frame" && m.t_ps >= 1500e9, 10_000, "frame at 1.5 s");
    const frames = session.frames(0).filter((f) => f.t_ps > 0);
    const levels = new Set(frames.map((f) => f.pins[d9]));
    check(`${id}: D9 is seen both high and low while it plays`, levels.has(0) && levels.has(1), [...levels]);
    const mean = (from, to) => {
      const span = frames.filter((f) => f.t_ps >= from && f.t_ps < to).map((f) => f.led_a[led]);
      return span.reduce((a, b) => a + b, 0) / span.length;
    };
    const early = mean(20e9, 300e9);
    const late = mean(1200e9, 1500e9);
    check(`${id}: PWM on D9 fades @led1 up, its mean current from ${early.toExponential(2)} A to ${late.toExponential(2)} A`, late > 2 * early && late > 1e-3, { early, late });
  } catch (error) {
    check(`${id}: ${error.message}`, false, session.seen.slice(-3));
  } finally {
    await session.close();
  }
}

async function refused() {
  const block = await loadBlock(index, "nano-button-led", readBytes);
  const files = new Map([...block.files].filter(([path]) => !path.startsWith("parts/led-5mm/")));
  const session = open({ name: block.name, files });
  try {
    const closed = await session.until((m) => m.type === "closed", 10_000, "closed");
    check(
      "a Block missing a part closes with the refusal `orbe open` prints",
      closed.reason.includes("error[part_missing]: part `led-5mm` is not in the catalog this Block reads"),
      closed
    );
  } catch (error) {
    check(`refusal: ${error.message}`, false, session.seen);
  } finally {
    await session.close();
  }
}

/** Wall milliseconds for one simulated second of `id`, ticked as fast as the thread runs. */
async function speed(id) {
  const block = await loadBlock(index, id, readBytes);
  const code = `
    const { parentPort, workerData } = require("node:worker_threads");
    const { readFileSync } = require("node:fs");
    (async () => {
      const { initSync, Block, Session } = await import(new URL("orbe_wasm.js", workerData.pkg));
      initSync({ module: readFileSync(new URL("orbe_wasm_bg.wasm", workerData.pkg)) });
      const runs = [];
      for (let run = 0; run < 3; run += 1) {
        const block = new Block();
        for (const [path, bytes] of workerData.files) block.add(path, bytes);
        const session = Session.open("speed", block);
        session.command(JSON.stringify({ id: 1, cmd: "play" }));
        session.messages();
        const start = performance.now();
        for (let ms = 0; ms < 1000; ms += 16) {
          session.tick(Math.min(16, 1000 - ms));
          session.messages();
        }
        runs.push(performance.now() - start);
        session.free();
      }
      parentPort.postMessage(runs);
    })();
  `;
  const worker = new Worker(code, { eval: true, workerData: { pkg: pkg.href, files: [...block.files] } });
  const runs = await new Promise((done, fail) => {
    worker.once("message", done);
    worker.once("error", fail);
  });
  await worker.terminate();
  const sorted = [...runs].sort((a, b) => a - b);
  console.log(`${id}: one simulated second takes ${sorted.map((ms) => ms.toFixed(0)).join(", ")} ms of wall time in a worker thread (median ${sorted[1].toFixed(0)} ms)`);
}

async function fixtureFiles() {
  const dir = resolve("packages/session/tests/light-divider");
  const names = await readdir(dir);
  const files = [];
  for (const name of names) files.push([name, new Uint8Array(await readFile(join(dir, name)))]);
  return files;
}

/** `worker.js` accepts `set` and the next frame carries the new light. */
async function lightInWorker() {
  const session = open({ name: "light", files: await fixtureFiles() });
  try {
    const scene = await session.until((m) => m.type === "scene", 10_000, "scene");
    const frame = await session.until((m) => m.type === "frame", 10_000, "frame");
    check("worker: the scene declares light and the first frame reads 300 lx", scene.world?.[0]?.name === "light" && frame.world?.[0] === 300, { scene: scene.world, frame: frame.world });
    await session.send({ cmd: "play" });
    const set = await session.send({ cmd: "set", quantity: "light", value: 50 });
    const landed = set.data?.landed_ps ?? 0;
    const after = await session.until(
      (m) => m.type === "frame" && m.t_ps > landed && m.world?.[0] === 50,
      5_000,
      "frame after set light 50"
    );
    check("worker: a frame past the landing reads 50 lx", set.ok === true && after.world[0] === 50, { set, world: after.world });
  } catch (error) {
    check(`worker light: ${error.message}`, false, session.seen.slice(-4));
  } finally {
    await session.close();
  }
}

/** The main thread uses `Session.command` directly, the same call a page makes outside the Worker. */
async function lightOnMainThread() {
  const { readFileSync } = await import("node:fs");
  const { initSync, Block, Session } = await import(new URL("orbe_wasm.js", pkg));
  initSync({ module: readFileSync(new URL("orbe_wasm_bg.wasm", pkg)) });
  const block = new Block();
  for (const [path, bytes] of await fixtureFiles()) block.add(path, bytes);
  const session = Session.open("light", block);
  const opened = session.messages().map((text) => JSON.parse(text));
  const first = opened.find((message) => message.type === "frame");
  session.command(JSON.stringify({ id: 1, cmd: "play" }));
  session.messages();
  session.command(JSON.stringify({ id: 2, cmd: "set", quantity: "light", value: 50 }));
  const answered = session.messages().map((text) => JSON.parse(text));
  const reply = answered.find((message) => message.type === "reply" && message.id === 2);
  const landed = reply?.reply?.data?.landed_ps ?? 0;
  session.tick(16);
  const after = session.messages().map((text) => JSON.parse(text));
  const frame = after.find((message) => message.type === "frame" && message.t_ps > landed);
  session.free();
  check(
    "main thread: set light 50 is on the next frame",
    first?.world?.[0] === 300 && reply?.reply?.ok === true && frame?.world?.[0] === 50 && frame.t_ps > landed,
    { first: first?.world, reply: reply?.reply, frame: frame?.world, t: frame?.t_ps, landed }
  );
}

await buttonLed("nano-button-led", "nano");
await buttonLed("uno-button-led", "uno", "@uno.led_l");
await fade();
await refused();
await lightInWorker();
await lightOnMainThread();
await speed("nano-button-led");
console.log(failed === 0 ? "all worker checks passed" : `${failed} worker checks failed`);
process.exit(failed === 0 ? 0 : 1);
