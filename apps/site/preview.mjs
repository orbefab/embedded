// Serves dist/ on 127.0.0.1 as the Worker's static assets do with wrangler.jsonc's settings: a
// folder's index.html at its path with a trailing slash, a redirect to that path without one,
// and the nearest 404.html for anything else. It reads no _headers. `wrangler dev` serves the
// same folder with them. Usage: node preview.mjs [--port <n>], or import `preview`.
import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, extname, join, resolve } from "node:path";

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".wasm": "application/wasm",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".ttf": "font/ttf",
  ".txt": "text/plain; charset=utf-8",
  ".md": "text/markdown; charset=utf-8",
};

const isFile = async (path) => (await stat(path).catch(() => null))?.isFile() ?? false;

/** Starts the server on `dir`, and resolves to its base URL and how to close it. */
export async function preview(dir = resolve(import.meta.dirname, "dist"), port = 0) {
  const send = (response, status, path) => {
    response.writeHead(status, { "Content-Type": TYPES[extname(path)] ?? "application/octet-stream" });
    createReadStream(path).pipe(response);
  };
  const server = createServer(async (request, response) => {
    const path = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
    const file = join(dir, path);
    if (!file.startsWith(dir)) return response.writeHead(400).end();
    if (path.endsWith("/") && (await isFile(join(file, "index.html")))) return send(response, 200, join(file, "index.html"));
    if (!path.endsWith("/") && (await isFile(file))) return send(response, 200, file);
    if (!path.endsWith("/") && (await isFile(join(file, "index.html")))) return response.writeHead(307, { Location: `${path}/` }).end();
    for (let at = path.endsWith("/") ? file : dirname(file); at.startsWith(dir); at = dirname(at)) {
      if (await isFile(join(at, "404.html"))) return send(response, 404, join(at, "404.html"));
    }
    response.writeHead(404).end();
  });
  await new Promise((done) => server.listen(port, "127.0.0.1", done));
  return { url: `http://127.0.0.1:${server.address().port}`, close: () => new Promise((done) => server.close(done)) };
}

if (import.meta.main) {
  const at = process.argv.indexOf("--port");
  const { url } = await preview(undefined, at < 0 ? 4173 : Number(process.argv[at + 1]));
  console.log(`${url}/embedded/`);
}
