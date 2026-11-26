// The pictures on the parts list's and the Blocks list's cards: one WebP per listed part, drawn
// ahead of time from the part's own 3D model, and one per Block, drawn from its scene as it runs
// about a second in. They are kept in apps/site/pictures with manifest.json, which keys each
// picture to what it was drawn from (build/pictures.ts says how).
//
// Run it after scripts/site.sh, whenever a release tag adds a part or a Block, changes a part's
// model or publishes a Block's new version, which is when `scripts/check-site.sh` says a picture
// is missing or stale:
//
//   scripts/site.sh
//   pnpm --filter @orbe/site pictures            # draws each missing or stale picture
//   pnpm --filter @orbe/site pictures -- --all   # draws every picture again
//
// It reads the registry and each part's view.json the build wrote under .scratch/site, draws each
// part in headless Chromium (found as the browser checks find it), and drops the picture of a
// part or Block the site no longer lists. A Block runs in orbe-wasm's Worker, the engine the
// Block page plays, at real speed or slower, and its picture is the first frame at or past its
// moment. A Block that has not reached it within BLOCK_LIMIT_MS of wall time fails the run. On an
// unchanged catalog it draws nothing and changes no file.
// `check` changes nothing and exits 1 while a picture is missing or stale; check-site.sh runs it.
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { readCatalog } from "../build/catalog.ts";
import { PUBLIC } from "../build/inputs.ts";
import { MANIFEST, manifestText, PICTURES, pictureFile, pictureProblems, readManifest, RENDER_COMMAND, wantedPictures } from "../build/pictures.ts";
import { at } from "../src/paths.ts";

const REPO = resolve(import.meta.dirname, "../../..");
const REGISTRY = join(REPO, ".scratch/site/registry");
const LAUNCH_ARGS = ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"];
/** How long a Block may run, in wall time, to reach the moment its picture shows. */
const BLOCK_LIMIT_MS = 300_000;

/** What picture-page.tsx puts on `window`. */
interface PicturePage {
  drawPart(viewUrl: string): Promise<string>;
  drawBlock(id: string, atMs: number, press: string | null, limitMs: number): Promise<{ picture: string; t_ps: number }>;
}

function fail(message: string): never {
  console.error(`pictures: ${message}`);
  process.exit(1);
}

function catalog() {
  if (!existsSync(join(REGISTRY, "index.json"))) fail(`no registry at ${REGISTRY}. Run scripts/site.sh first.`);
  return readCatalog(REGISTRY);
}

function check(): void {
  const problems = pictureProblems(catalog());
  if (problems.length === 0) {
    console.log("pictures: every listed part and every Block has a current picture");
    return;
  }
  for (const line of problems) console.error(`pictures: ${line}`);
  fail(`draw them again with \`scripts/site.sh && ${RENDER_COMMAND}\`, then commit apps/site/pictures.`);
}

async function render(all: boolean): Promise<void> {
  const wanted = wantedPictures(catalog());
  const manifest = readManifest();
  const names = new Set(wanted.map((each) => each.name));
  for (const name of Object.keys(manifest)) {
    if (names.has(name)) continue;
    delete manifest[name];
    rmSync(pictureFile(name), { force: true });
    console.log(`pictures: dropped ${name}, which the site no longer lists`);
  }
  const todo = wanted.filter((each) => all || manifest[each.name] !== each.key || !existsSync(pictureFile(each.name)));
  for (const each of todo) {
    if (each.kind === "part" && !existsSync(join(PUBLIC, "parts", each.id, "view.json"))) fail(`no view of ${each.id} in ${PUBLIC}. Run scripts/site.sh first.`);
    if (each.kind === "block" && !existsSync(join(PUBLIC, "orbe-wasm", "worker.js"))) fail(`no orbe-wasm in ${PUBLIC}. Run scripts/site.sh first.`);
  }
  mkdirSync(join(PICTURES, "blocks"), { recursive: true });
  if (todo.length > 0) {
    const { createServer } = await import("vite");
    const { default: react } = await import("@vitejs/plugin-react");
    const { chromium } = await import("@playwright/test");
    const page = "picture.html";
    const server = await createServer({
      configFile: false,
      root: import.meta.dirname,
      base: "/orbe/",
      publicDir: PUBLIC,
      logLevel: "error",
      plugins: [
        react(),
        {
          name: "orbe-picture-page",
          configureServer(vite) {
            vite.middlewares.use(`/orbe/${page}`, (_request, response) => {
              const html = `<!doctype html><html><body style="margin:0"><div id="root"></div><script type="module" src="/picture-page.tsx"></script></body></html>`;
              void vite.transformIndexHtml(`/orbe/${page}`, html).then((text) => {
                response.setHeader("Content-Type", "text/html");
                response.end(text);
              });
            });
          },
        },
      ],
      server: { host: "127.0.0.1", port: 0 },
    });
    await server.listen();
    const address = server.httpServer?.address();
    if (address === null || address === undefined || typeof address === "string") fail("the page server has no port");
    const executablePath = process.env.ORBE_CHROMIUM || chromium.executablePath();
    if (!existsSync(executablePath)) fail(`no Chromium at ${executablePath}. Set ORBE_CHROMIUM, or run \`pnpm exec playwright install chromium\`.`);
    const browser = await chromium.launch({ executablePath, args: LAUNCH_ARGS });
    try {
      const tab = await browser.newPage();
      const errors: string[] = [];
      tab.on("pageerror", (error) => errors.push(error.message));
      await tab.goto(`http://127.0.0.1:${address.port}/orbe/${page}`);
      await tab.waitForFunction(() => "drawPart" in window, undefined, { timeout: 60_000 });
      for (const each of todo) {
        let data: string;
        let shown = "";
        if (each.kind === "part") {
          data = await tab.evaluate((view) => (window as unknown as PicturePage).drawPart(view), at.partView(each.id));
        } else {
          const started = Date.now();
          const { atMs, press } = each.moment;
          const drawn = await tab
            .evaluate(([id, at, held, limit]) => (window as unknown as PicturePage).drawBlock(id, at, held, limit), [each.id, atMs, press, BLOCK_LIMIT_MS] as const)
            .catch((error: unknown) => fail(`Block ${each.id}: ${error instanceof Error ? error.message.split("\n")[0] : String(error)}`));
          data = drawn.picture;
          const held = press === null ? "" : `, holding ${press}`;
          shown = ` at t = ${(drawn.t_ps / 1e12).toFixed(3)} s${held}, after ${((Date.now() - started) / 1000).toFixed(1)} s of wall time`;
        }
        if (errors.length > 0) fail(`${each.name}: ${errors.join("; ")}`);
        const prefix = "data:image/webp;base64,";
        if (!data.startsWith(prefix)) fail(`${each.name}: the page returned no WebP`);
        writeFileSync(pictureFile(each.name), Buffer.from(data.slice(prefix.length), "base64"));
        manifest[each.name] = each.key;
        console.log(`pictures: drew ${each.name}${shown}`);
      }
    } finally {
      await browser.close();
      await server.close();
    }
  }
  const text = manifestText(manifest);
  if (!existsSync(MANIFEST) || text !== manifestText(readManifest())) writeFileSync(MANIFEST, text);
  console.log(`pictures: ${todo.length} drawn, ${wanted.length - todo.length} already current`);
}

const [command, ...rest] = process.argv.slice(2).filter((arg) => arg !== "--");
if (command === "check" && rest.length === 0) check();
else if (command === "render" && (rest.length === 0 || (rest.length === 1 && rest[0] === "--all"))) await render(rest[0] === "--all");
else {
  console.error("usage: node apps/site/scripts/pictures.ts render [--all] | check");
  process.exit(2);
}
