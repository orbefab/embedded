// The parts list's pictures: one WebP per listed part, drawn ahead of time from the part's own 3D
// model, kept in apps/site/pictures with manifest.json, which keys each picture to the models it
// was drawn from (build/pictures.ts says how).
//
// Run it after scripts/site.sh, whenever a release tag adds a part or changes a part's model,
// which is when `scripts/check-site.sh` says a picture is missing or stale:
//
//   scripts/site.sh
//   pnpm --filter @orbe/site pictures            # draws each missing or stale picture
//   pnpm --filter @orbe/site pictures -- --all   # draws every picture again
//
// It reads the registry and each part's view.json the build wrote under .scratch/site, draws each
// part in headless Chromium (found as the browser checks find it), and drops the picture of a
// part the site no longer lists. On an unchanged catalog it draws nothing and changes no file.
// `check` changes nothing and exits 1 while a picture is missing or stale; check-site.sh runs it.
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { readCatalog } from "../build/catalog.ts";
import { PUBLIC } from "../build/inputs.ts";
import {
  listedParts,
  MANIFEST,
  manifestText,
  PICTURES,
  pictureFile,
  pictureKey,
  pictureProblems,
  readManifest,
  RENDER_COMMAND,
} from "../build/pictures.ts";
import { at } from "../src/paths.ts";

const REPO = resolve(import.meta.dirname, "../../..");
const REGISTRY = join(REPO, ".scratch/site/registry");
const LAUNCH_ARGS = ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"];

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
    console.log("pictures: every listed part has a current picture");
    return;
  }
  for (const line of problems) console.error(`pictures: ${line}`);
  fail(`draw them again with \`scripts/site.sh && ${RENDER_COMMAND}\`, then commit apps/site/pictures.`);
}

async function render(all: boolean): Promise<void> {
  const parts = catalog();
  const listed = listedParts(parts);
  const manifest = readManifest();
  const wanted = new Set(listed);
  for (const id of Object.keys(manifest)) {
    if (wanted.has(id)) continue;
    delete manifest[id];
    rmSync(pictureFile(id), { force: true });
    console.log(`pictures: dropped ${id}, which the site no longer lists`);
  }
  const todo = listed.flatMap((id) => {
    const key = pictureKey(parts, id);
    if (key === null) return [];
    if (!all && manifest[id] === key && existsSync(pictureFile(id))) return [];
    if (!existsSync(join(PUBLIC, "parts", id, "view.json"))) fail(`no view of ${id} in ${PUBLIC}. Run scripts/site.sh first.`);
    return [{ id, key }];
  });
  mkdirSync(PICTURES, { recursive: true });
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
      for (const { id, key } of todo) {
        const url = at.partView(id);
        const data = await tab.evaluate((view) => (window as unknown as { drawPart(url: string): Promise<string> }).drawPart(view), url);
        if (errors.length > 0) fail(`${id}: ${errors.join("; ")}`);
        const prefix = "data:image/webp;base64,";
        if (!data.startsWith(prefix)) fail(`${id}: the page returned no WebP`);
        writeFileSync(pictureFile(id), Buffer.from(data.slice(prefix.length), "base64"));
        manifest[id] = key;
        console.log(`pictures: drew ${id}`);
      }
    } finally {
      await browser.close();
      await server.close();
    }
  }
  const text = manifestText(manifest);
  if (!existsSync(MANIFEST) || text !== manifestText(readManifest())) writeFileSync(MANIFEST, text);
  console.log(`pictures: ${todo.length} drawn, ${listed.length - todo.length} already current`);
}

const [command, ...rest] = process.argv.slice(2).filter((arg) => arg !== "--");
if (command === "check" && rest.length === 0) check();
else if (command === "render" && (rest.length === 0 || (rest.length === 1 && rest[0] === "--all"))) await render(rest[0] === "--all");
else {
  console.error("usage: node apps/site/scripts/pictures.ts render [--all] | check");
  process.exit(2);
}
