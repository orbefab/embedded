// Drives the built site in headless Chromium: the landing page plays a Block and frees its
// Worker when another Block is shown, the parts search finds what `orbe search` finds, a part's
// page and a board's page draw their 3D and list files, licences and sellers, and a Block's page
// lights the LED on a press. It prints page weight and the time to the first Block frame.
// scripts/check-site.sh builds the site first.
// Usage: node site.mjs --orbe <orbe binary> [--base <url>] [--renders <dir>]
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { gzipSync } from "node:zlib";
import { chromium } from "playwright-core";
import { preview } from "../preview.mjs";

const arg = (name) => {
  const at = process.argv.indexOf(name);
  return at < 0 ? undefined : process.argv[at + 1];
};
const orbe = arg("--orbe");
const renders = arg("--renders");
if (orbe === undefined) {
  console.error("usage: node site.mjs --orbe <orbe binary> [--base <url>] [--renders <dir>]");
  process.exit(2);
}
if (renders !== undefined) mkdirSync(renders, { recursive: true });
const executable = process.env.ORBE_CHROMIUM ?? chromium.executablePath();
if (!existsSync(executable)) {
  console.error(`No Chromium at ${executable}. This script installs no browser. Set ORBE_CHROMIUM, or run`);
  console.error("`pnpm --filter @orbe/site exec playwright-core install chromium`.");
  process.exit(2);
}
if (arg("--base") === undefined && !existsSync(resolve(import.meta.dirname, "../dist/embedded/index.html"))) {
  console.error("No apps/site/dist. Run scripts/site.sh first, or scripts/check-site.sh.");
  process.exit(2);
}

const server = arg("--base") === undefined ? await preview() : null;
const base = arg("--base") ?? server.url;
const browser = await chromium.launch({ executablePath: executable, args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] });

let failed = 0;
function check(name, pass, observed) {
  if (!pass) failed += 1;
  console.log(`${pass ? "ok  " : "FAIL"} ${name}`);
  if (!pass || process.env.ORBE_E2E_VERBOSE) console.log(`     observed: ${JSON.stringify(observed)}`);
}
const shot = async (page, name) => {
  if (renders !== undefined) await page.screenshot({ path: `${renders}/${name}.png`, fullPage: false });
};
const INSTALL = /orbe add|cargo install|github\.com\/orbefab\/embedded/;

/** A page whose Workers and object URLs are tracked, so a check can see what a switch frees. */
async function open(path) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [];
  const fetched = [];
  page.on("requestfinished", (request) =>
    fetched.push(
      request
        .response()
        .then((response) => response?.body())
        .then((body) => [request.url(), body?.length ?? 0, body === undefined ? 0 : gzipSync(body, { level: 6 }).length])
        .catch(() => [request.url(), 0, 0])
    )
  );
  page.weight = async () => {
    const sizes = await Promise.all(fetched);
    const sum = (at) => sizes.reduce((total, size) => total + size[at], 0);
    return { requests: sizes.length, bytes: sum(1), gzip: sum(2), wasm: sizes.some(([url]) => url.endsWith(".wasm")) };
  };
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.addInitScript(() => {
    const workers = [];
    const created = [];
    const revoked = new Set();
    const Original = window.Worker;
    window.Worker = class extends Original {
      constructor(...args) {
        super(...args);
        this.created = created.length;
        this.terminated = false;
        workers.push(this);
      }
      terminate() {
        this.terminated = true;
        super.terminate();
      }
    };
    const create = URL.createObjectURL.bind(URL);
    const revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = (blob) => {
      const url = create(blob);
      created.push(url);
      return url;
    };
    URL.revokeObjectURL = (url) => {
      revoked.add(url);
      revoke(url);
    };
    window.orbeTrack = { workers, created, revoked };
  });
  await page.goto(`${base}${path}`);
  return { page, errors };
}

const object = (page, ref) => page.evaluate((ref) => window.orbeViewer?.get().scene.getObjectByName(ref) !== undefined, ref);
const waitObject = (page, ref, timeout = 30_000) =>
  page.waitForFunction((ref) => window.orbeViewer?.get().scene.getObjectByName(ref) !== undefined, ref, { timeout });
const text = (page) => page.evaluate(() => document.body.innerText);
const glow = (page) =>
  page.evaluate(() => window.orbeViewer.get().scene.getObjectByName("@led1")?.getObjectByName("body")?.material.emissiveIntensity ?? null);
const pointOn = (page, ref, node) =>
  page.evaluate(
    ([ref, node]) => {
      const { scene, camera, size } = window.orbeViewer.get();
      let box;
      scene
        .getObjectByName(ref)
        .getObjectByName(node)
        .traverse((mesh) => {
          if (!mesh.isMesh) return;
          mesh.geometry.computeBoundingBox();
          const own = mesh.geometry.boundingBox.clone().applyMatrix4(mesh.matrixWorld);
          box = box === undefined ? own : box.union(own);
        });
      const at = box.getCenter(box.min.clone()).project(camera);
      const canvas = document.querySelector("canvas").getBoundingClientRect();
      return { x: canvas.left + ((at.x + 1) / 2) * size.width, y: canvas.top + ((1 - at.y) / 2) * size.height };
    },
    [ref, node]
  );
const markAt = (page, name) => page.evaluate((name) => performance.getEntriesByName(name)[0]?.startTime ?? null, name);

async function step(name, run) {
  try {
    await run();
  } catch (error) {
    check(`${name}: a step threw`, false, String(error?.message ?? error).split("\n")[0]);
  }
}

await step("landing", async () => {
  const { page, errors } = await open("/embedded/");
  check("the landing page says what orbe is", (await page.textContent("h1")) === "orbe", await page.textContent("h1"));
  await waitObject(page, "@btn1");
  await page.waitForFunction(() => performance.getEntriesByName("orbe:first-frame:nano-button-led").length > 0, null, { timeout: 30_000 });
  await page.waitForFunction(() => document.querySelector('[data-testid="state"]')?.textContent === "playing", null, { timeout: 10_000 });
  check("the first Block plays on the landing page", true, null);
  const first = await markAt(page, "orbe:first-frame:nano-button-led");
  await page.waitForTimeout(500);
  const loaded = await page.weight();
  console.log(`     landing: first Block frame at ${Math.round(first)} ms; ${loaded.requests} requests, ${loaded.bytes} bytes, ${loaded.gzip} gzipped, the Worker's wasm ${loaded.wasm ? "included" : "missing"}`);
  check("the landing page shows no install command", !INSTALL.test(await text(page)), (await text(page)).match(INSTALL)?.[0]);
  check("it says the CLI is coming", (await page.locator('[data-testid="cli-coming"]').count()) === 1, null);
  await shot(page, "site-landing");

  await page.click('[data-block="nano-led-fade"]');
  await page.waitForFunction(() => performance.getEntriesByName("orbe:first-frame:nano-led-fade").length > 0, null, { timeout: 30_000 });
  const freed = await page.evaluate(() => {
    const { workers, created, revoked } = window.orbeTrack;
    const [one, two] = workers;
    const before = created.slice(0, two.created);
    return {
      workers: workers.length,
      firstTerminated: one.terminated,
      secondTerminated: two.terminated,
      firstUrls: before.length,
      firstRevoked: before.filter((url) => revoked.has(url)).length,
      secondRevoked: created.slice(two.created).filter((url) => revoked.has(url)).length,
    };
  });
  check(
    "switching to nano-led-fade terminates the first Worker and revokes every object URL it made",
    freed.workers === 2 && freed.firstTerminated && !freed.secondTerminated && freed.firstUrls > 0 && freed.firstRevoked === freed.firstUrls && freed.secondRevoked === 0,
    freed
  );
  await page.waitForFunction(() => document.querySelector('[data-testid="state"]')?.textContent === "playing", null, { timeout: 10_000 });
  check("nano-led-fade plays after the switch", true, null);
  await page.waitForTimeout(1500);
  await shot(page, "site-landing-switched");
  check("the landing page logged no error", errors.length === 0, errors);
  await page.close();
});

await step("parts search", async () => {
  const { page, errors } = await open("/embedded/parts/");
  const all = await page.locator("[data-part]").count();
  check("the parts page lists every part without a query", all === 45, all);
  for (const query of ["led", "5 mm", "kingbright", "board", "resistor 0603", "smd", "avr usb", "nano-board", "Push", "MICROCHIP"]) {
    await page.fill('input[name="q"]', query);
    const shown = await page.locator("[data-part]").evaluateAll((items) => items.map((item) => item.dataset.part));
    const cli = JSON.parse(execFileSync(orbe, ["search", ...query.split(" "), "--json"], { encoding: "utf8" })).data.parts.map((part) => part.name);
    check(`"${query}" finds what orbe search finds, in its order`, JSON.stringify(shown) === JSON.stringify(cli), { shown, cli });
  }
  await page.fill('input[name="q"]', "led");
  const kept = await page.evaluate(() => new URL(window.location.href).searchParams.get("q"));
  check("the query is in the URL", kept === "led", kept);
  await shot(page, "site-parts-search-led");
  check("the parts page logged no error", errors.length === 0, errors);
  await page.close();
});

await step("led-5mm", async () => {
  const { page, errors } = await open("/embedded/parts/led-5mm/");
  await waitObject(page, "@led_5mm");
  check("led-5mm's page draws its model", await object(page, "@led_5mm"), null);
  const body = await text(page);
  check("it says Orbefab is not affiliated with Kingbright", /not affiliated with or endorsed by Kingbright/.test(body), null);
  const files = await page.locator("section a[download]").evaluateAll((links) => links.map((link) => [link.textContent, link.getAttribute("href")]));
  check("it lists all five files as downloads", files.length === 5 && files.some(([path]) => path === "led-5mm.glb"), files);
  const licences = await page.locator("section", { has: page.locator("h2", { hasText: /^Files$/ }) }).locator("td:nth-child(3)").allTextContents();
  check("every file shows its licence", licences.length === 5 && licences.every((licence) => licence === "MIT OR Apache-2.0"), licences);
  const download = await page.evaluate(async (href) => {
    const record = await (await fetch("/embedded/r/led-5mm@0.1.0.json")).json();
    const bytes = new Uint8Array(await (await fetch(href)).arrayBuffer());
    const hex = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map((b) => b.toString(16).padStart(2, "0")).join("");
    return { hex, want: record.files.find((file) => file.path === "part.jsonc").sha256 };
  }, files.find(([path]) => path === "part.jsonc")[1]);
  check("a download serves the file's bytes, by the record's SHA-256", download.hex === download.want, download);
  check("it lists DigiKey with its checked date", /DigiKey\s*2026-11-08/.test(body), body.match(/Where to buy[\s\S]{0,120}/i)?.[0]);
  check("it shows the ratings with their sources", /30 mA/.test(body) && /Absolute Maximum Ratings/.test(body), null);
  check("it shows what is not simulated", /reverse breakdown/.test(body), null);
  check("it shows no install command", !INSTALL.test(body), body.match(INSTALL)?.[0]);
  await page.waitForTimeout(800);
  const loaded = await page.weight();
  console.log(`     led-5mm: ${loaded.requests} requests, ${loaded.bytes} bytes, ${loaded.gzip} gzipped`);
  await shot(page, "site-part-led-5mm");
  await page.evaluate(() => [...document.querySelectorAll("h2")].find((h) => h.textContent === "Files")?.scrollIntoView());
  await shot(page, "site-part-led-5mm-files");
  check("led-5mm's page logged no error", errors.length === 0, errors);
  await page.close();
});

await step("nano-board", async () => {
  const { page, errors } = await open("/embedded/parts/nano-board/");
  await waitObject(page, "@nano_board.mcu");
  check("the Nano's page draws the board with its children", await object(page, "@nano_board.usb_serial"), null);
  const body = await text(page);
  check("it is titled as a clone", (await page.textContent("h1")) === "Nano (CH340 clone)", await page.textContent("h1"));
  check("it says Orbefab is not affiliated with Arduino", /not affiliated with or endorsed by Arduino/.test(body), null);
  check("it lists the parts it is built from", /Built from[\s\S]*CH340G/i.test(body), null);
  check("it lists the Blocks that use it", /Used by[\s\S]*Button and LED on a Nano/i.test(body), null);
  await page.waitForTimeout(800);
  await shot(page, "site-part-nano-board");
  check("the Nano's page logged no error", errors.length === 0, errors);
  await page.close();
  const uno = await open("/embedded/parts/uno-board/");
  check("the Uno is titled as a compatible board", (await uno.page.textContent("h1")) === "Uno R3 compatible board", await uno.page.textContent("h1"));
  await uno.page.close();
});

await step("nano-button-led", async () => {
  const { page, errors } = await open("/embedded/blocks/nano-button-led/");
  await waitObject(page, "@btn1");
  await page.waitForFunction(() => document.querySelector('[data-testid="state"]')?.textContent === "playing", null, { timeout: 10_000 });
  check("the Block's page plays it", true, null);
  await page.locator("[data-island]").scrollIntoViewIfNeeded();
  const stem = await pointOn(page, "@btn1", "stem");
  await page.mouse.move(stem.x, stem.y);
  await page.mouse.down();
  await page.waitForFunction(
    () => window.orbeViewer.get().scene.getObjectByName("@led1").getObjectByName("body").material.emissiveIntensity > 0.5,
    null,
    { timeout: 10_000 }
  );
  check("held, the button lights @led1", (await glow(page)) > 0.5, await glow(page));
  await shot(page, "site-block-pressed-lit");
  await page.mouse.up();
  await page.waitForFunction(
    () => window.orbeViewer.get().scene.getObjectByName("@led1").getObjectByName("body").material.emissiveIntensity < 0.05,
    null,
    { timeout: 10_000 }
  );
  check("released, @led1 goes dark", (await glow(page)) < 0.05, await glow(page));
  const body = await text(page);
  check("the page shows the scene and the firmware source", /scene\.jsonc/.test(body) && /firmware\/button\/button\.ino/.test(body), null);
  check("it shows no install command and no README", !INSTALL.test(body) && !/README\.md\n/.test(body.split("Files")[0] ?? ""), body.match(INSTALL)?.[0]);
  check("the session refused no command", (await page.locator('[data-testid="errors"]').count()) === 0, await page.locator('[data-testid="errors"]').allTextContents());
  check("the Block's page logged no error", errors.length === 0, errors);
  await page.close();
});

await step("not found", async () => {
  const page = await browser.newPage();
  const response = await page.goto(`${base}/embedded/no-such-page`);
  check("an unknown path answers 404 with the site's page", response.status() === 404 && (await page.textContent("h1")) === "Not here", response.status());
  await page.close();
});

await browser.close();
await server?.close();
console.log(failed === 0 ? "all site checks passed" : `${failed} site checks failed`);
process.exit(failed === 0 ? 0 : 1);
