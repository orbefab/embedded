// Starts the session `orbe open --headed --no-browser` and stays up so Playwright's webServer
// can own it. The CLI exits after the session process is listening, and an exited command is a
// dead server, so this process waits until SIGTERM and then closes the session.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sessionPath = join(repo, ".scratch/headed/session.json");
const orbe = process.env.ORBE ?? "";
const runtime = mkdtempSync(join(tmpdir(), "orbe-headed-"));
const env = { ...process.env, XDG_RUNTIME_DIR: runtime };
delete env.ORBE_SESSION;
delete env.ORBE_VIEWER_DIR;

function call(...args) {
  try {
    return JSON.parse(execFileSync(orbe, [...args, "--json"], { cwd: repo, env, encoding: "utf8" }));
  } catch (error) {
    if (typeof error.stdout === "string" && error.stdout.startsWith("{")) return JSON.parse(error.stdout);
    return { ok: false, error: { message: String(error?.message ?? error) } };
  }
}

let opened = { ok: false, error: { message: `No orbe at ${orbe}` }, data: {} };
if (orbe !== "" && existsSync(orbe)) {
  opened = call("open", "blocks/nano-button-led/scene.jsonc", "--headed", "--no-browser");
}
mkdirSync(dirname(sessionPath), { recursive: true });
writeFileSync(
  sessionPath,
  JSON.stringify({
    ok: Boolean(opened.ok),
    url: opened.data?.url ?? null,
    error: opened.error ?? null,
    state: opened.state ?? null,
    runtime,
  }),
);
console.log("ORBE_SHOW_READY=1");

const stay = setInterval(() => {}, 1 << 30);
let closed = false;
function shutdown() {
  if (closed) return;
  closed = true;
  clearInterval(stay);
  try {
    call("close");
  } catch {
    // `orbe close` refuses when the test already closed the session.
  }
  rmSync(runtime, { recursive: true, force: true });
  process.exit(0);
}
process.on("SIGTERM", shutdown);
process.on("SIGINT", shutdown);
