#!/usr/bin/env bash
# Proves scripts/site-previous.sh. orbe builds a small registry and a fixture server on
# port 4173 serves it. Exit codes, redirects, a stalled body and an unsafe URL are the
# downloader's. A downloaded file with one byte changed is refused by `orbe registry build
# --previous`.
set -euo pipefail
cd "$(dirname "$0")/.."

if curl -sS -o /dev/null --max-time 1 "http://127.0.0.1:4173/" 2>/dev/null; then
  echo "scripts/site-previous.test.sh: port 4173 is already in use" >&2
  exit 1
fi

work=$(mktemp -d)
server_pid=
cleanup() {
  if [[ -n ${server_pid:-} ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill -TERM -"$server_pid" 2>/dev/null || kill -TERM "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

expect_code() {
  local want=$1
  shift
  local err=$work/err
  set +e
  "$@" >"$work/out" 2>"$err"
  local code=$?
  set -e
  if [[ $code -ne $want ]]; then
    echo "scripts/site-previous.test.sh: $* exited $code, wanted $want" >&2
    cat "$err" >&2
    exit 1
  fi
  cat "$err"
}

expect_fail() {
  local needle=$1
  shift
  local err
  err=$(expect_code 1 "$@")
  if [[ -n $needle ]] && ! grep -q "$needle" <<<"$err"; then
    echo "scripts/site-previous.test.sh: the failure did not mention $needle" >&2
    printf '%s\n' "$err" >&2
    exit 1
  fi
}

stop_server() {
  if [[ -n ${server_pid:-} ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill -TERM -"$server_pid" 2>/dev/null || kill -TERM "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  server_pid=
}

start_group() {
  set -m
  "$@" >"$work/server.log" 2>&1 &
  server_pid=$!
  set +m
}

part() {
  local root=$1 id=$2
  mkdir -p "$root/parts/$id"
  printf '%s\n' '{ "format": 1, "name": "@acme" }' >"$root/registry.jsonc"
  cat >"$root/parts/$id/part.jsonc" <<EOF
{
  "format": 1,
  "part": "$id",
  "version": "1.2.0",
  "title": "Lamp",
  "license": "MIT",
  "pins": { "a": [1], "b": [2] },
  "params": { "resistance": { "unit": "ohm", "default": "220", "min_exclusive": 0, "sets": "/resistance" } },
  "model": { "kind": "resistor", "ratings": { "power": null } }
}
EOF
}

same() {
  local got=$1 want=$2 file rel
  while IFS= read -r -d '' file; do
    rel=${file#"$want/"}
    [[ $rel == *.schema.json ]] && continue
    cmp -s "$file" "$got/$rel" || {
      echo "scripts/site-previous.test.sh: $rel differs" >&2
      exit 1
    }
  done < <(find "$want" -type f -print0)
  while IFS= read -r -d '' file; do
    rel=${file#"$got/"}
    if [[ ! -f $want/$rel ]]; then
      echo "scripts/site-previous.test.sh: $rel is an extra download" >&2
      exit 1
    fi
  done < <(find "$got" -type f -print0)
}

echo "scripts/site-previous.test.sh: usage"
expect_code 2 scripts/site-previous.sh
expect_code 2 scripts/site-previous.sh --url http://127.0.0.1:4173/embedded/r/
expect_code 2 scripts/site-previous.sh --not-a-flag
expect_code 2 scripts/site-previous.sh --allow-first --url http://127.0.0.1:4173/embedded/r/ --out "$work/nope"

echo "scripts/site-previous.test.sh: orbe"
export CARGO_BUILD_JOBS=4
cargo build -p orbe --locked
target=$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
orbe=$target/debug/orbe
part "$work/src" lamp
part "$work/next" wire
"$orbe" registry build "$work/src" "$work/www/r"

cat >"$work/serve.mjs" <<'EOF'
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { appendFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const root = process.argv[2];
const mode = process.argv[3];
const ready = process.argv[4];
const hits = process.argv[5];
const server = createServer(async (request, response) => {
  if (hits) appendFileSync(hits, `${request.url ?? ""}\n`);
  const url = new URL(request.url ?? "/", "http://127.0.0.1:4173");
  if (mode === "stall" && url.pathname.endsWith("/index.json")) {
    response.writeHead(200, { "content-type": "application/json" });
    response.write('{"items":[');
    return;
  }
  if (mode === "hop" && url.pathname.endsWith("/index.json")) {
    response.writeHead(307, { location: "/embedded/r/hop.json" });
    response.end();
    return;
  }
  if (mode === "hop" && url.pathname.endsWith("/hop.json")) {
    const body = await readFile(join(root, "r/index.json"));
    response.writeHead(200, { "content-type": "application/json" });
    response.end(body);
    return;
  }
  if (mode === "unsafe" && url.pathname.endsWith("/index.json")) {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({
      items: [{ name: "lamp", version: "1.0.0", url: "../../etc/passwd" }],
    }));
    return;
  }
  if (mode === "off-host" && url.pathname.includes("/files/")) {
    response.writeHead(307, { Location: "http://127.0.0.1:1/secret" });
    response.end();
    return;
  }
  if (mode === "missing-index" && url.pathname.endsWith("/index.json")) {
    response.writeHead(404);
    response.end();
    return;
  }
  if (mode === "status-500" && url.pathname.endsWith("/index.json")) {
    response.writeHead(500);
    response.end();
    return;
  }
  if (url.pathname.includes("@")) {
    response.writeHead(307, { Location: url.pathname.replaceAll("@", "%40") });
    response.end();
    return;
  }
  const path = decodeURIComponent(url.pathname);
  const prefix = "/embedded/";
  if (!path.startsWith(prefix)) {
    response.writeHead(404);
    response.end();
    return;
  }
  try {
    const body = await readFile(join(root, path.slice(prefix.length)));
    response.writeHead(200);
    response.end(body);
  } catch {
    response.writeHead(404);
    response.end();
  }
});
server.listen(4173, "127.0.0.1", () => {
  if (ready) writeFileSync(ready, "ready\n");
});
EOF

serve() {
  local mode=$1
  stop_server
  rm -f "$work/ready"
  start_group node "$work/serve.mjs" "$work/www" "$mode" "$work/ready" "$work/hits"
  local _
  for _ in $(seq 1 50); do
    if [[ $mode == stall ]]; then
      [[ -f $work/ready ]] && return 0
    elif curl -sS -o /dev/null --max-time 1 "http://127.0.0.1:4173/embedded/r/index.json" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  echo "scripts/site-previous.test.sh: the fixture server did not answer" >&2
  cat "$work/server.log" >&2
  exit 1
}

url=http://127.0.0.1:4173/embedded/r/
echo "scripts/site-previous.test.sh: records are requested with %40"
serve redirect
: >"$work/hits"
scripts/site-previous.sh --url "$url" --out "$work/redirected"
same "$work/redirected" "$work/www/r"
grep -q '%40' "$work/hits"
if grep -q '@' "$work/hits"; then
  echo "scripts/site-previous.test.sh: a record URL was requested with a raw @" >&2
  cat "$work/hits" >&2
  exit 1
fi

echo "scripts/site-previous.test.sh: trailing slash"
scripts/site-previous.sh --url "$url" --out "$work/slash/"
same "$work/slash" "$work/www/r"
[[ -f $work/slash/index.json ]]
if find "$work" -maxdepth 1 -name 'slash.partial-*' | grep -q .; then
  echo "scripts/site-previous.test.sh: a partial directory was left behind" >&2
  exit 1
fi

echo "scripts/site-previous.test.sh: a non-empty out is left in place"
mkdir -p "$work/occupied"
echo keep >"$work/occupied/keep.txt"
expect_fail "not empty" scripts/site-previous.sh --url "$url" --out "$work/occupied"
[[ $(cat "$work/occupied/keep.txt") == keep ]]
expect_fail "not empty" scripts/site-previous.sh --url "$url" --out "$work/occupied/"
[[ $(cat "$work/occupied/keep.txt") == keep ]]

echo "scripts/site-previous.test.sh: redirect inside the registry"
serve hop
scripts/site-previous.sh --url "$url" --out "$work/hopped"
same "$work/hopped" "$work/www/r"

echo "scripts/site-previous.test.sh: a tampered file is refused by the registry build"
"$orbe" registry build "$work/next" "$work/carried" --previous "$work/redirected" >"$work/carry.out"
sample=$(find "$work/redirected/files" -type f -print -quit)
[[ -n $sample ]]
printf x >>"$sample"
expect_fail "does not match the hash and size" "$orbe" registry build "$work/next" "$work/refused" --previous "$work/redirected"
[[ ! -e $work/refused ]]

echo "scripts/site-previous.test.sh: redirect outside the registry"
serve off-host
expect_fail "outside" scripts/site-previous.sh --url "$url" --out "$work/outside"
[[ ! -e $work/outside ]]

echo "scripts/site-previous.test.sh: unsafe url"
serve unsafe
expect_fail "../../etc/passwd" scripts/site-previous.sh --url "$url" --out "$work/unsafe"
[[ ! -e $work/unsafe ]]

echo "scripts/site-previous.test.sh: a 404 is an error and is not retried"
serve missing-index
: >"$work/hits"
expect_fail "answered 404" scripts/site-previous.sh --url "$url" --out "$work/missing"
[[ ! -e $work/missing ]]
[[ $(grep -c 'index.json' "$work/hits" || true) -eq 1 ]]

echo "scripts/site-previous.test.sh: a 500 is retried"
serve status-500
: >"$work/hits"
expect_fail "answered 500" scripts/site-previous.sh --url "$url" --out "$work/down"
[[ $(grep -c 'index.json' "$work/hits" || true) -eq 3 ]]
stop_server

echo "scripts/site-previous.test.sh: a stalled body"
serve stall
: >"$work/hits"
stall_err=$(SITE_PREVIOUS_TIMEOUT_MS=1000 expect_code 1 scripts/site-previous.sh --url "$url" --out "$work/stall")
if ! grep -q "could not fetch" <<<"$stall_err"; then
  echo "scripts/site-previous.test.sh: the stalled body did not name the fetch" >&2
  printf '%s\n' "$stall_err" >&2
  exit 1
fi
if ! grep -q "retry 2 of 3" <<<"$stall_err" || ! grep -q "retry 3 of 3" <<<"$stall_err"; then
  echo "scripts/site-previous.test.sh: the stalled body did not retry" >&2
  printf '%s\n' "$stall_err" >&2
  exit 1
fi
if grep -Eq 'DOMException|node:internal|TimeoutError|^    at ' <<<"$stall_err"; then
  echo "scripts/site-previous.test.sh: the stalled body dumped a stack" >&2
  printf '%s\n' "$stall_err" >&2
  exit 1
fi
[[ $(grep -c 'index.json' "$work/hits" || true) -eq 3 ]]
[[ ! -e $work/stall ]]
stop_server

echo "scripts/site-previous.test.sh: network error"
expect_fail "could not fetch" scripts/site-previous.sh --url "$url" --out "$work/offline"
echo "scripts/site-previous.test.sh: ok"
