// Builds the site into dist/, the folder the Worker serves: every page under dist/embedded/, the
// registry at dist/embedded/r/, orbe-wasm at dist/embedded/orbe-wasm/, and the headers. The
// registry and the package come from this same commit, as scripts/site.sh builds them.
// Usage: node build.mjs --registry <dir> --wasm <dir>. ORBE_SITE_INSTALL=1 shows install lines.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cp, mkdir, readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { build } from "vite";

const APP = import.meta.dirname;
const REPO = resolve(APP, "../..");
const OUT = resolve(APP, "dist");
const SSR = resolve(APP, "node_modules/.site-ssr");

const arg = (name) => {
  const at = process.argv.indexOf(name);
  return at < 0 ? undefined : process.argv[at + 1];
};
const registry = arg("--registry");
const wasm = arg("--wasm");
if (registry === undefined || wasm === undefined) {
  console.error("usage: node build.mjs --registry <dir> --wasm <dir>. scripts/site.sh builds both from this commit, then runs this.");
  process.exit(2);
}
const flag = process.env.ORBE_SITE_INSTALL ?? "";
if (!["", "0", "1"].includes(flag)) {
  console.error(`ORBE_SITE_INSTALL is ${JSON.stringify(flag)}. Set it to 1 to show install lines, or leave it unset.`);
  process.exit(2);
}

execFileSync(resolve(REPO, "scripts/brand.sh"), ["check"], { stdio: "inherit" });

await rm(OUT, { recursive: true, force: true });
const client = join(OUT, "embedded");
await build({
  root: APP,
  logLevel: "warn",
  build: { outDir: client, emptyOutDir: true, manifest: true, rollupOptions: { input: resolve(APP, "src/client.tsx") } },
});
const manifest = JSON.parse(await readFile(join(client, ".vite/manifest.json"), "utf8"));
await rm(join(client, ".vite"), { recursive: true });
const entry = manifest["src/client.tsx"];
const assets = { script: `/embedded/${entry.file}`, styles: (entry.css ?? []).map((file) => `/embedded/${file}`) };

await build({
  root: APP,
  logLevel: "warn",
  publicDir: false,
  build: { ssr: resolve(APP, "src/render.tsx"), outDir: SSR, emptyOutDir: true },
});
const site = await import(pathToFileURL(join(SSR, "render.js")).href);

const engine = JSON.parse(await readFile(join(wasm, "package.json"), "utf8"));
const orbeWasm = await import(pathToFileURL(resolve(wasm, "orbe_wasm.js")).href);
orbeWasm.initSync({ module: await readFile(join(wasm, "orbe_wasm_bg.wasm")) });
const draw = (id, files) => {
  const block = new orbeWasm.Block();
  for (const [path, bytes] of files) block.add(path, bytes);
  return orbeWasm.partScene(id, block);
};

const catalog = site.readCatalog(registry);
const settings = {
  install: flag === "1",
  registry: catalog.name,
  engine: { version: engine.version, commit: engine.orbe?.commit ?? null },
};
const outputs = [
  ...site.renderPages(catalog, settings, assets),
  ...site.partViews(catalog, draw),
  { file: "_headers", text: site.headers },
];
for (const { file, text } of outputs) {
  await mkdir(dirname(join(OUT, file)), { recursive: true });
  await writeFile(join(OUT, file), text);
}
await cp(registry, join(client, "r"), { recursive: true });
for (const file of engine.files) await cp(join(wasm, file), join(client, "orbe-wasm", file));
await cp(resolve(REPO, "packages/brand/OFL.txt"), join(client, "fonts/OFL.txt"));
await rm(SSR, { recursive: true });

async function* walk(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(path);
    else yield path;
  }
}
const groups = new Map();
const hashes = new Set();
let files = 0;
let bytes = 0;
for await (const path of walk(OUT)) {
  const size = (await stat(path)).size;
  const rel = relative(OUT, path);
  const group = /^embedded\/(r|orbe-wasm|assets)\//.exec(rel)?.[1] ?? (rel.endsWith(".html") ? "pages" : "other");
  const [count, total] = groups.get(group) ?? [0, 0];
  groups.set(group, [count + 1, total + size]);
  hashes.add(createHash("sha256").update(await readFile(path)).digest("hex"));
  files += 1;
  bytes += size;
}
console.log(`site: wrote ${relative(process.cwd(), OUT) || "."}: ${files} files, ${bytes} bytes, ${hashes.size} distinct by SHA-256`);
for (const [group, [count, total]] of [...groups].sort()) console.log(`  ${group.padEnd(9)} ${String(count).padStart(4)} files ${String(total).padStart(9)} bytes`);
console.log(`  install lines ${settings.install ? "shown" : "hidden, the CLI is coming"}`);
const commit = settings.engine.commit;
console.log(`  orbe-wasm ${settings.engine.version} from ${commit ?? "no commit"}`);
if (commit === null || commit.endsWith("-dirty")) console.warn("site: warning: orbe-wasm was not built from a clean commit. Deploy a build of a clean checkout.");
