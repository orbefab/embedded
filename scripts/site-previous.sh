#!/usr/bin/env bash
# Downloads the registry a site serves into a directory for `scripts/site.sh --previous`.
# Fetches index.json, each record, and each files/<sha256> the record lists. A row URL must
# be <name>@<version>.json with no /, .. or \ in the name or version. A file URL must be
# files/ plus 64 lowercase hex. Redirects stay in that directory. `orbe registry build
# --previous` verifies everything it reads. A network error, a timeout, or a status other
# than 200 exits 1, and nothing is written on failure. `--allow-first`: index.json answering
# 404 exits 10. A 404 without it, or a 200 with it, exits 1.
#
# Usage: scripts/site-previous.sh --url <base> --out <dir> [--allow-first]
# <base> is http or https, with no user, query or fragment. Needs Node 24.
set -euo pipefail

usage() {
  cat <<'EOF'
usage: scripts/site-previous.sh --url <base> --out <dir> [--allow-first]

Download the registry at <base> into <dir>. orbe registry build --previous verifies
everything it reads. --allow-first: index.json answering 404 exits 10. A 200 exits 1.
EOF
}

url=
out=
allow_first=0
die() { echo "scripts/site-previous.sh: $*" >&2; usage >&2; exit 2; }
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h | --help) usage; exit 0 ;;
    --allow-first) allow_first=1; shift ;;
    --url) [[ $# -ge 2 && -n ${2:-} && ${2:-} != --* ]] || die "--url needs the registry URL"; url=$2; shift 2 ;;
    --out) [[ $# -ge 2 && -n ${2:-} && ${2:-} != --* ]] || die "--out needs a directory"; out=$2; shift 2 ;;
    *) die "unknown argument $1" ;;
  esac
done
[[ -n $url && -n $out ]] || die "--url and --out are required"
export SITE_PREVIOUS_URL=$url SITE_PREVIOUS_OUT=$out SITE_PREVIOUS_ALLOW_FIRST=$allow_first

node --input-type=module <<'END'
import { mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve, sep } from "node:path";

const MAX_BYTES = 64 * 1024 * 1024;
const allowFirst = process.env.SITE_PREVIOUS_ALLOW_FIRST === "1";
const out = process.env.SITE_PREVIOUS_OUT;
let stage;

function fail(message, code = 1) {
  console.error(`scripts/site-previous.sh: ${message}`);
  if (stage !== undefined) rmSync(stage, { recursive: true, force: true });
  process.exit(code);
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

async function get(url, base) {
  let current = url;
  for (let hop = 0; hop < 5; hop += 1) {
    if (!inside(current, base)) return fail(`${url} redirected to ${current}, outside ${base}`);
    let response;
    try { response = await fetch(current, { redirect: "manual", signal: AbortSignal.timeout(60_000) }); }
    catch (error) { return fail(`could not fetch ${current}: ${error instanceof Error ? error.message : error}`); }
    if (response.status >= 300 && response.status < 400) {
      const location = response.headers.get("location");
      await response.arrayBuffer().catch(() => {});
      if (!location) return fail(`${current} redirected with no Location`);
      current = new URL(location, current).href;
      continue;
    }
    const body = Buffer.from(await response.arrayBuffer());
    if (body.length > MAX_BYTES) return fail(`${current} is ${body.length} bytes, and a registry file is at most ${MAX_BYTES} bytes`);
    return { status: response.status, body };
  }
  return fail(`${url} redirected too many times`);
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

const base = baseUrl(process.env.SITE_PREVIOUS_URL);
const indexUrl = new URL("index.json", base).href;
const index = await get(indexUrl, base);
if (index.status === 404 && allowFirst) {
  console.log("scripts/site-previous.sh: index.json answered 404, so this is the first deploy");
  process.exit(10);
}
if (index.status !== 200) fail(`index.json answered ${index.status}`);
if (allowFirst) fail("index.json answered 200 and --allow-first is set. Unset SITE_FIRST_DEPLOY once the site serves a registry.");

const document = jsonObject(index.body, "index.json");
if (!Array.isArray(document.items)) fail("index.json has no items");
const saved = new Map([["index.json", index.body]]);

async function take(rel) {
  if (saved.has(rel)) return saved.get(rel);
  const fetched = await get(new URL(rel, indexUrl).href, base);
  if (fetched.status !== 200) fail(`${rel} answered ${fetched.status}`);
  saved.set(rel, fetched.body);
  return fetched.body;
}

for (const row of document.items) {
  const rel = `${row?.name}@${row?.version}.json`;
  if (row === null || typeof row !== "object" || !token(row.name) || !token(row.version) || row.url !== rel) {
    fail(`index.json gives ${JSON.stringify(row?.name)}@${JSON.stringify(row?.version)} the URL ${JSON.stringify(row?.url)}, and a record's URL is <name>@<version>.json`);
  }
  const record = jsonObject(await take(rel), rel);
  if (!Array.isArray(record.files)) fail(`${rel} has no files`);
  for (const file of record.files) {
    if (file === null || typeof file !== "object" || !fileUrl(file.url)) {
      fail(`${rel} lists a file at ${JSON.stringify(file?.url)}, and a file's URL is files/<sha256>`);
    }
    await take(file.url);
  }
}

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
  rmSync(out, { recursive: true, force: true });
  renameSync(stage, out);
  stage = undefined;
} catch (error) {
  fail(`could not write ${out}: ${error instanceof Error ? error.message : error}`);
}
console.log(`scripts/site-previous.sh: wrote ${saved.size} files to ${out}`);
END
