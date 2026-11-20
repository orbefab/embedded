#!/usr/bin/env bash
# Downloads the registry a site serves into a directory for `scripts/site.sh --previous`.
# Fetches index.json, each record, and each files/<sha256> the record lists. A record is
# requested with @ encoded as %40. A row URL must be <name>@<version>.json with no /, .. or
# \ in the name or version. A file URL must be files/ plus 64 lowercase hex. Redirects stay
# in that directory. Eight requests run at a time. A network error, a timeout or a 5xx is
# tried 3 times. A 404 is an error and is not retried. A bad entry fails the whole download,
# and nothing is written on failure. --out is stripped of trailing separators and must be
# missing or empty. `orbe registry build --previous` verifies everything it reads.
#
# Usage: scripts/site-previous.sh --url <base> --out <dir>
# <base> is http or https, with no user, query or fragment. Needs Node 24.
# SITE_PREVIOUS_TIMEOUT_MS overrides the per-request budget. The test sets it.
set -euo pipefail

usage() {
  cat <<'EOF'
usage: scripts/site-previous.sh --url <base> --out <dir>

Download the registry at <base> into <dir>. <dir> must be missing or empty. orbe registry
build --previous verifies everything it reads. A 404, a bad entry, or a redirect outside
<base> fails the download and writes nothing.
EOF
}

url=
out=
die() { echo "scripts/site-previous.sh: $*" >&2; usage >&2; exit 2; }
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h | --help) usage; exit 0 ;;
    --url) [[ $# -ge 2 && -n ${2:-} && ${2:-} != --* ]] || die "--url needs the registry URL"; url=$2; shift 2 ;;
    --out) [[ $# -ge 2 && -n ${2:-} && ${2:-} != --* ]] || die "--out needs a directory"; out=$2; shift 2 ;;
    *) die "unknown argument $1" ;;
  esac
done
[[ -n $url && -n $out ]] || die "--url and --out are required"
export SITE_PREVIOUS_URL=$url SITE_PREVIOUS_OUT=$out

env -u FORCE_COLOR node --input-type=module <<'END'
import { mkdirSync, readdirSync, renameSync, rmdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve, sep } from "node:path";

const ATTEMPTS = 3;
const POOL = 8;
let stage;
let stopping = false;

function fail(message, code = 1) {
  if (!stopping) {
    stopping = true;
    console.error(`scripts/site-previous.sh: ${message}`);
    if (stage !== undefined) rmSync(stage, { recursive: true, force: true });
  }
  process.exit(code);
}

process.on("unhandledRejection", (error) => {
  fail(error instanceof Error ? error.message : String(error));
});

function timeoutMs() {
  const raw = process.env.SITE_PREVIOUS_TIMEOUT_MS;
  if (raw === undefined || raw === "") return 60_000;
  if (!/^[1-9][0-9]*$/.test(raw)) fail("SITE_PREVIOUS_TIMEOUT_MS is not a positive number of milliseconds");
  return Number(raw);
}

const budget = timeoutMs();

function outputDir(raw) {
  if (typeof raw !== "string") fail("--out names no directory");
  const stripped = raw.replace(/[/\\]+$/, "");
  if (stripped === "") fail("--out names no directory");
  return stripped;
}

function outState(dir) {
  try {
    const entries = readdirSync(dir);
    if (entries.length > 0) fail(`${dir} is not empty, and a registry is written into an empty folder`);
    return "empty";
  } catch (error) {
    if (error?.code === "ENOENT") return "missing";
    fail(`could not read ${dir}: ${error instanceof Error ? error.message : error}`);
  }
}

function baseUrl(raw) {
  let url;
  try { url = new URL(raw); } catch { return fail(`not a URL: ${raw}`); }
  if (url.username || url.password) return fail("the registry URL must not include credentials");
  if (url.search || url.hash) return fail("the registry URL must not include a query or a fragment");
  if (url.protocol !== "http:" && url.protocol !== "https:") return fail("the registry URL must be http or https");
  if (!url.pathname.endsWith("/")) url.pathname += "/";
  return url.href;
}

function inside(href, base) {
  let url;
  try { url = new URL(href); } catch { return false; }
  const root = new URL(base);
  if (url.origin !== root.origin || url.username || url.password || url.search || url.hash || /%2e|%2f|\\/i.test(url.pathname)) return false;
  try { return decodeURIComponent(url.pathname).startsWith(decodeURIComponent(root.pathname)); }
  catch { return false; }
}

function errorText(error) {
  if (!(error instanceof Error)) return String(error);
  const cause = error.cause;
  const extra = cause instanceof Error ? cause.message : typeof cause === "string" ? cause : "";
  return extra !== "" && !error.message.includes(extra) ? `${error.message}: ${extra}` : error.message;
}

function pause(ms) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, ms));
}

async function request(url, base) {
  let current = url;
  for (let hop = 0; hop < 5; hop += 1) {
    if (!inside(current, base)) return { retry: false, error: `${url} redirected to ${current}, outside ${base}` };
    let status;
    try {
      const response = await fetch(current, { redirect: "manual", signal: AbortSignal.timeout(budget) });
      status = response.status;
      if (status >= 300 && status < 400) {
        const location = response.headers.get("location");
        await response.arrayBuffer();
        if (!location) return { retry: false, error: `${current} redirected with no Location` };
        current = new URL(location, current).href;
        continue;
      }
      const body = Buffer.from(await response.arrayBuffer());
      return { status, body };
    } catch (error) {
      const retry = !(status >= 400 && status < 500);
      return { retry, error: `could not fetch ${current}: ${errorText(error)}` };
    }
  }
  return { retry: false, error: `${url} redirected too many times` };
}

async function get(url, base) {
  for (let attempt = 1; attempt <= ATTEMPTS; attempt += 1) {
    const result = await request(url, base);
    if (result.error) {
      if (!result.retry || attempt === ATTEMPTS) fail(result.error);
      console.error(`scripts/site-previous.sh: retry ${attempt + 1} of ${ATTEMPTS} for ${url}`);
      await pause(attempt * 1000);
      continue;
    }
    if (result.status >= 500 && attempt < ATTEMPTS) {
      console.error(`scripts/site-previous.sh: retry ${attempt + 1} of ${ATTEMPTS} for ${url}`);
      await pause(attempt * 1000);
      continue;
    }
    return result;
  }
  fail(`could not fetch ${url}`);
}

const token = (value) => typeof value === "string" && value !== "" && !value.includes("/") && !value.includes("\\") && !value.includes("..");
const fileUrl = (value) => typeof value === "string" && /^files\/[0-9a-f]{64}$/.test(value);

function jsonObject(body, label) {
  let value;
  try { value = JSON.parse(body.toString("utf8")); }
  catch (error) { return fail(`${label} is not JSON: ${error instanceof Error ? error.message : error}`); }
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail(`${label} is not a JSON object`);
  return value;
}

const out = outputDir(process.env.SITE_PREVIOUS_OUT);
outState(out);
const base = baseUrl(process.env.SITE_PREVIOUS_URL);
const indexUrl = new URL("index.json", base).href;
const index = await get(indexUrl, base);
if (index.status !== 200) fail(`index.json answered ${index.status}`);

const document = jsonObject(index.body, "index.json");
if (!Array.isArray(document.items)) fail("index.json has no items");
const saved = new Map([["index.json", index.body]]);

async function take(rel) {
  if (saved.has(rel)) return saved.get(rel);
  const fetched = await get(new URL(rel.replaceAll("@", "%40"), indexUrl).href, base);
  if (fetched.status !== 200) fail(`${rel} answered ${fetched.status}`);
  saved.set(rel, fetched.body);
  return fetched.body;
}

async function pool(rels) {
  const queue = [...rels];
  const width = Math.min(POOL, queue.length);
  if (width === 0) return;
  await Promise.all(Array.from({ length: width }, async () => {
    for (;;) {
      const rel = queue.shift();
      if (rel === undefined) return;
      await take(rel);
    }
  }));
}

const records = [];
const seenRecords = new Set();
for (const row of document.items) {
  const rel = `${row?.name}@${row?.version}.json`;
  if (row === null || typeof row !== "object" || !token(row.name) || !token(row.version) || row.url !== rel) {
    fail(`index.json gives ${JSON.stringify(row?.name)}@${JSON.stringify(row?.version)} the URL ${JSON.stringify(row?.url)}, and a record's URL is <name>@<version>.json`);
  }
  if (seenRecords.has(rel)) continue;
  seenRecords.add(rel);
  records.push(rel);
}
await pool(records);

const files = [];
const seenFiles = new Set();
for (const rel of records) {
  const record = jsonObject(saved.get(rel), rel);
  if (!Array.isArray(record.files)) fail(`${rel} has no files`);
  for (const file of record.files) {
    if (file === null || typeof file !== "object" || !fileUrl(file.url)) {
      fail(`${rel} lists a file at ${JSON.stringify(file?.url)}, and a file's URL is files/<sha256>`);
    }
    if (seenFiles.has(file.url)) continue;
    seenFiles.add(file.url);
    files.push(file.url);
  }
}
await pool(files);

stage = `${out}.partial-${process.pid}`;
rmSync(stage, { recursive: true, force: true });
mkdirSync(stage, { recursive: true });
const root = resolve(stage) + sep;
try {
  for (const [rel, body] of saved) {
    const dest = resolve(stage, rel);
    if (!dest.startsWith(root)) fail(`${rel} is outside the output`);
    mkdirSync(dirname(dest), { recursive: true });
    writeFileSync(dest, body);
  }
  if (outState(out) === "empty") rmdirSync(out);
  renameSync(stage, out);
  stage = undefined;
} catch (error) {
  fail(`could not write ${out}: ${error instanceof Error ? error.message : error}`);
}
console.log(`scripts/site-previous.sh: wrote ${saved.size} files to ${out}`);
END
