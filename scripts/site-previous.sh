#!/usr/bin/env bash
# Downloads the registry a site serves into a directory for `scripts/site.sh --previous`.
# The registry is the one at <url>, which is https://orbefab.com/embedded/r/ for this site.
# It starts at index.json, then every record, schema and file that index names, and checks
# each record and file against the hash and size the registry states. docs/registry.md
# (Verify what you fetch, Versions) is the rule. A network error, a status other than 200,
# a redirect that leaves the registry, or a hash or size that does not match exits 1.
# Nothing is written until every check has passed.
#
# `--allow-first` is the first deploy. index.json answering 404 then exits 10 and writes
# nothing, which tells the caller to pass `--first-deploy`. A 404 without `--allow-first`
# exits 1. A 200 with `--allow-first` exits 1, so the flag is removed once a registry is up.
#
# Usage: scripts/site-previous.sh --url <base> --out <dir> [--allow-first]
# <base> is the registry directory URL, with or without a trailing slash. It is http or https,
# with no user, query or fragment. Needs Node 24. It installs nothing.
set -euo pipefail

usage() {
  cat <<'EOF'
usage: scripts/site-previous.sh --url <base> --out <dir> [--allow-first]

Download the registry served at <base> into <dir>, for scripts/site.sh --previous.
Each record and file is checked against its hash and size. A network error, a status
other than 200, or a mismatch exits 1.

--allow-first  The first deploy. index.json answering 404 exits 10 and writes nothing.
               A 200 with this flag exits 1.
EOF
}

url=
out=
allow_first=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h | --help)
      usage
      exit 0
      ;;
    --allow-first)
      allow_first=1
      shift
      ;;
    --url)
      if [[ $# -lt 2 || -z ${2:-} || ${2:-} == --* ]]; then
        echo "scripts/site-previous.sh: --url needs the registry URL" >&2
        usage >&2
        exit 2
      fi
      url=$2
      shift 2
      ;;
    --out)
      if [[ $# -lt 2 || -z ${2:-} || ${2:-} == --* ]]; then
        echo "scripts/site-previous.sh: --out needs a directory" >&2
        usage >&2
        exit 2
      fi
      out=$2
      shift 2
      ;;
    *)
      echo "scripts/site-previous.sh: unknown argument $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done
if [[ -z $url || -z $out ]]; then
  echo "scripts/site-previous.sh: --url and --out are required" >&2
  usage >&2
  exit 2
fi

export SITE_PREVIOUS_URL=$url
export SITE_PREVIOUS_OUT=$out
export SITE_PREVIOUS_ALLOW_FIRST=$allow_first

node --input-type=module <<'END'
import { createHash } from "node:crypto";
import { mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const MAX_BYTES = 64 * 1024 * 1024;
const allowFirst = process.env.SITE_PREVIOUS_ALLOW_FIRST === "1";
let stagePath;

function sha256(body) {
  return createHash("sha256").update(body).digest("hex");
}

function fail(message, code = 1) {
  console.error(`scripts/site-previous.sh: ${message}`);
  if (stagePath !== undefined) {
    try {
      rmSync(stagePath, { recursive: true, force: true });
    } catch {
      // The process is exiting. A leftover partial directory is removed on the next run.
    }
  }
  process.exit(code);
}

function normalizeBase(raw) {
  let url;
  try {
    url = new URL(raw);
  } catch {
    return fail(`not a URL: ${raw}`);
  }
  if (url.username !== "" || url.password !== "") return fail("the registry URL must not include credentials");
  if (url.search !== "" || url.hash !== "") return fail("the registry URL must not include a query or a fragment");
  if (url.protocol !== "http:" && url.protocol !== "https:") return fail("the registry URL must be http or https");
  if (!url.pathname.endsWith("/")) url.pathname += "/";
  return url.href;
}

function underBase(href, base) {
  let url;
  try {
    url = new URL(href);
  } catch {
    return false;
  }
  const root = new URL(base);
  if (url.origin !== root.origin || url.username !== "" || url.password !== "" || url.search !== "" || url.hash !== "") return false;
  if (/%2e|%2f|\\/i.test(url.pathname)) return false;
  try {
    return decodeURIComponent(url.pathname).startsWith(decodeURIComponent(root.pathname));
  } catch {
    return false;
  }
}

function relative(href, base) {
  if (!underBase(href, base)) return undefined;
  const path = decodeURIComponent(new URL(href).pathname);
  const prefix = decodeURIComponent(new URL(base).pathname);
  const rel = path.slice(prefix.length);
  const parts = rel.split("/");
  if (rel === "" || parts.some((part) => part === "" || part === "." || part === "..")) return undefined;
  return rel;
}

async function get(url, base) {
  let current = url;
  for (let hop = 0; hop < 5; hop += 1) {
    if (!underBase(current, base)) return fail(`${url} redirected to ${current}, outside ${base}`);
    let response;
    try {
      response = await fetch(current, { redirect: "manual", signal: AbortSignal.timeout(60_000) });
    } catch (error) {
      return fail(`could not fetch ${current}: ${error instanceof Error ? error.message : error}`);
    }
    if (response.status >= 300 && response.status < 400) {
      const location = response.headers.get("location");
      await response.arrayBuffer().catch(() => {});
      if (location === null || location === "") return fail(`${current} redirected with no Location`);
      current = new URL(location, current).href;
      continue;
    }
    const body = Buffer.from(await response.arrayBuffer());
    if (body.length > MAX_BYTES) return fail(`${current} is ${body.length} bytes, and a registry file is at most ${MAX_BYTES} bytes`);
    return { status: response.status, body };
  }
  return fail(`${url} redirected too many times`);
}

function hex(value) {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

function size(value) {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value <= MAX_BYTES;
}

const base = normalizeBase(process.env.SITE_PREVIOUS_URL);
const out = process.env.SITE_PREVIOUS_OUT;
const indexUrl = new URL("index.json", base).href;
const index = await get(indexUrl, base);
if (index.status === 404) {
  if (allowFirst) {
    console.log("scripts/site-previous.sh: index.json answered 404, so this is the first deploy");
    process.exit(10);
  }
  fail("index.json answered 404");
}
if (index.status !== 200) fail(`index.json answered ${index.status}`);
if (allowFirst) fail("index.json answered 200 and --allow-first is set. Unset SITE_FIRST_DEPLOY once the site serves a registry.");

let document;
try {
  document = JSON.parse(index.body.toString("utf8"));
} catch (error) {
  fail(`index.json is not JSON: ${error instanceof Error ? error.message : error}`);
}
if (document === null || typeof document !== "object" || Array.isArray(document)) fail("index.json is not a registry index");
if (document.format !== 1) fail(`index.json has format ${JSON.stringify(document.format)}, and this client knows format 1`);
if (document.name !== "@orbefab") fail(`index.json is the registry ${JSON.stringify(document.name)}, and this site serves @orbefab`);
if (!Array.isArray(document.items)) fail("index.json has no items");
if (typeof document.$schema !== "string" || document.$schema === "") fail("index.json has no schema");

const planned = new Map();
function plan(rel, fileUrl, expected) {
  const sha = expected === null ? null : expected.sha;
  const bytes = expected === null ? null : expected.bytes;
  const seen = planned.get(rel);
  if (seen !== undefined) {
    if (seen.url !== fileUrl || seen.sha !== sha || seen.bytes !== bytes) {
      fail(`${rel} is listed twice with different hashes`);
    }
    return;
  }
  planned.set(rel, { url: fileUrl, sha, bytes, body: null });
}

const indexRel = relative(indexUrl, base);
if (indexRel === undefined) fail(`${indexUrl} is outside ${base}`);
planned.set(indexRel, { url: indexUrl, sha: null, bytes: null, body: index.body });

function resolveRef(ref, against, label) {
  if (typeof ref !== "string" || ref === "" || ref.includes("\\") || ref.startsWith("/") || ref.includes("://")) {
    return fail(`${label} has the URL ${JSON.stringify(ref)}, which is not a path in this registry`);
  }
  let fileUrl;
  try {
    fileUrl = new URL(ref, against).href;
  } catch {
    return fail(`${label} has the URL ${JSON.stringify(ref)}`);
  }
  const rel = relative(fileUrl, base);
  if (rel === undefined) return fail(`${label} resolves to ${fileUrl}, outside ${base}`);
  return { url: fileUrl, rel };
}

const schema = resolveRef(document.$schema, indexUrl, "index.json");
plan(schema.rel, schema.url, null);

const seen = new Set();
for (const row of document.items) {
  if (row === null || typeof row !== "object") fail("index.json has an item that is not an object");
  const at = `${row.name}@${row.version}`;
  if (typeof row.name !== "string" || typeof row.version !== "string" || row.url !== `${at}.json`) {
    fail(`index.json gives ${JSON.stringify(row.name)}@${JSON.stringify(row.version)} the URL ${JSON.stringify(row.url)}, and a record's URL is ${at}.json`);
  }
  if (seen.has(at)) fail(`index.json lists ${at} twice`);
  seen.add(at);
  if (!hex(row.sha256) || !size(row.bytes)) fail(`index.json gives ${at} no hash and size`);
  const recordRef = resolveRef(row.url, indexUrl, at);
  plan(recordRef.rel, recordRef.url, { sha: row.sha256, bytes: row.bytes });
}

for (const [rel, file] of planned) {
  if (file.body !== null) continue;
  const fetched = await get(file.url, base);
  if (fetched.status !== 200) fail(`${rel} answered ${fetched.status}`);
  file.body = fetched.body;
  if (file.sha === null) {
    if (file.body.length === 0) fail(`${rel} is empty`);
    continue;
  }
  const got = sha256(file.body);
  if (got !== file.sha || file.body.length !== file.bytes) {
    fail(`${rel} has SHA-256 ${got} and ${file.body.length} bytes, and the registry says ${file.sha} and ${file.bytes} bytes`);
  }
}

for (const row of document.items) {
  const at = `${row.name}@${row.version}`;
  const recordRel = `${at}.json`;
  const record = planned.get(recordRel);
  let parsed;
  try {
    parsed = JSON.parse(record.body.toString("utf8"));
  } catch (error) {
    fail(`${recordRel} is not JSON: ${error instanceof Error ? error.message : error}`);
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) fail(`${recordRel} is not a record`);
  if (parsed.format !== 1) fail(`${recordRel} has format ${JSON.stringify(parsed.format)}`);
  if (parsed.name !== row.name || parsed.version !== row.version || parsed.type !== row.type) {
    fail(`${recordRel} is not the ${row.type} ${at} its index row names`);
  }
  if (!Array.isArray(parsed.files)) fail(`${recordRel} has no files`);
  if (typeof parsed.$schema !== "string") fail(`${recordRel} has no schema`);
  const recordSchema = resolveRef(parsed.$schema, record.url, recordRel);
  plan(recordSchema.rel, recordSchema.url, null);
  const paths = new Set();
  const lines = [];
  for (const file of parsed.files) {
    if (file === null || typeof file !== "object") fail(`${recordRel} has a file that is not an object`);
    if (typeof file.path !== "string" || file.path === "") fail(`${recordRel} lists a file with no path`);
    if (paths.has(file.path)) fail(`${recordRel} lists ${file.path} twice`);
    paths.add(file.path);
    if (!hex(file.sha256) || !size(file.bytes) || file.url !== `files/${file.sha256}`) {
      fail(`${recordRel} lists ${file.path} at ${JSON.stringify(file.url)}, and a file's URL is files/<sha256>`);
    }
    const ref = resolveRef(file.url, record.url, `${recordRel} ${file.path}`);
    plan(ref.rel, ref.url, { sha: file.sha256, bytes: file.bytes });
    lines.push(`${file.sha256}  ${file.path}\n`);
  }
  lines.sort((a, b) => (a.slice(66) < b.slice(66) ? -1 : a.slice(66) > b.slice(66) ? 1 : 0));
  const folder = `sha256:${sha256(Buffer.from(lines.join(""), "utf8"))}`;
  if (parsed.hash !== folder) fail(`${recordRel} has folder hash ${JSON.stringify(parsed.hash)}, and its files hash to ${folder}`);
}

for (const [rel, file] of planned) {
  if (file.body !== null) continue;
  const fetched = await get(file.url, base);
  if (fetched.status !== 200) fail(`${rel} answered ${fetched.status}`);
  const got = sha256(fetched.body);
  if (file.sha === null) {
    if (fetched.body.length === 0) fail(`${rel} is empty`);
  } else if (got !== file.sha || fetched.body.length !== file.bytes) {
    fail(`${rel} has SHA-256 ${got} and ${fetched.body.length} bytes, and the registry says ${file.sha} and ${file.bytes} bytes`);
  }
  file.body = fetched.body;
}

stagePath = `${out}.partial-${process.pid}`;
rmSync(stagePath, { recursive: true, force: true });
mkdirSync(stagePath, { recursive: true });
for (const [rel, file] of planned) {
  const path = join(stagePath, rel);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, file.body);
}
rmSync(out, { recursive: true, force: true });
try {
  renameSync(stagePath, out);
} catch (error) {
  fail(`could not write ${out}: ${error instanceof Error ? error.message : error}`);
}
stagePath = undefined;
console.log(`scripts/site-previous.sh: wrote ${planned.size} files to ${out}`);
END
