#!/usr/bin/env bash
# Proves scripts/site-previous.sh against the registry in apps/site/dist/client/r.
# The happy path, a tampered file and a missing file go through vite preview on port 4173.
# A second server on that same port, started after preview stops, answers 307 for @,
# a redirect off the registry, 404 and 500. Needs the site build and a free port 4173.
set -euo pipefail
cd "$(dirname "$0")/.."

registry=apps/site/dist/client/r
[[ -f $registry/index.json ]] || {
  echo "scripts/site-previous.test.sh: build the site first (scripts/site.sh --first-deploy)" >&2
  exit 1
}
if curl -sS -o /dev/null --max-time 1 "http://127.0.0.1:4173/"; then
  echo "scripts/site-previous.test.sh: port 4173 is already in use" >&2
  exit 1
fi

work=$(mktemp -d)
server_pid=
orig=
orig_target=
cleanup() {
  [[ -z ${orig:-} ]] || cp -a "$orig" "$orig_target" || true
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

wait_url() {
  local url=$1
  local _
  for _ in $(seq 1 150); do
    if curl -fsS -o /dev/null --max-time 1 "$url"; then
      return 0
    fi
    if ! kill -0 "$server_pid" 2>/dev/null; then
      echo "scripts/site-previous.test.sh: the server exited" >&2
      cat "$work/server.log" >&2
      exit 1
    fi
    sleep 0.2
  done
  echo "scripts/site-previous.test.sh: $url did not answer" >&2
  cat "$work/server.log" >&2
  exit 1
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

echo "scripts/site-previous.test.sh: usage"
expect_code 2 scripts/site-previous.sh
expect_code 2 scripts/site-previous.sh --url http://127.0.0.1:4173/embedded/r/
expect_code 2 scripts/site-previous.sh --not-a-flag

url=http://127.0.0.1:4173/embedded/r/
echo "scripts/site-previous.test.sh: vite preview"
start_group pnpm --dir apps/site exec vite preview --strictPort --host 127.0.0.1 --port 4173
wait_url "$url/index.json"
scripts/site-previous.sh --url "$url" --out "$work/preview"
diff -rq "$registry" "$work/preview"
expect_fail "answered 200 and --allow-first" scripts/site-previous.sh --allow-first --url "$url" --out "$work/allow-live"
[[ ! -e $work/allow-live ]]

sample=$(find "$registry/files" -type f -print -quit)
[[ -n $sample ]]
orig=$(mktemp)
orig_target=$sample
cp -a "$sample" "$orig"
printf x >>"$sample"
stop_server
start_group pnpm --dir apps/site exec vite preview --strictPort --host 127.0.0.1 --port 4173
wait_url "$url/index.json"
expect_fail "SHA-256" scripts/site-previous.sh --url "$url" --out "$work/tampered"
[[ ! -e $work/tampered ]]
cp -a "$orig" "$sample"

rm -f "$sample"
stop_server
start_group pnpm --dir apps/site exec vite preview --strictPort --host 127.0.0.1 --port 4173
wait_url "$url/index.json"
expect_fail "$(basename "$sample")" scripts/site-previous.sh --url "$url" --out "$work/missing"
[[ ! -e $work/missing ]]
cp -a "$orig" "$sample"
rm -f "$orig"
orig=
orig_target=
stop_server

cat >"$work/serve.mjs" <<'EOF'
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const root = process.argv[2];
const mode = process.argv[3];
const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", "http://127.0.0.1:4173");
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
server.listen(4173, "127.0.0.1");
EOF

serve() {
  local mode=$1
  stop_server
  start_group node "$work/serve.mjs" apps/site/dist/client "$mode"
  local _
  for _ in $(seq 1 50); do
    if curl -sS -o /dev/null --max-time 1 "http://127.0.0.1:4173/embedded/r/index.json"; then
      return 0
    fi
    if [[ $mode == missing-index || $mode == status-500 ]]; then
      local code
      code=$(curl -sS -o /dev/null --max-time 1 -w '%{http_code}' "http://127.0.0.1:4173/embedded/r/index.json" || true)
      if [[ $code == 404 || $code == 500 ]]; then
        return 0
      fi
    fi
    sleep 0.1
  done
  echo "scripts/site-previous.test.sh: the fixture server did not answer" >&2
  cat "$work/server.log" >&2
  exit 1
}

echo "scripts/site-previous.test.sh: redirect @ to %40"
serve redirect
scripts/site-previous.sh --url "$url" --out "$work/redirected"
diff -rq "$registry" "$work/redirected"

echo "scripts/site-previous.test.sh: redirect outside the registry"
serve off-host
expect_fail "outside" scripts/site-previous.sh --url "$url" --out "$work/outside"

echo "scripts/site-previous.test.sh: first deploy"
serve missing-index
expect_code 10 scripts/site-previous.sh --allow-first --url "$url" --out "$work/first"
[[ ! -e $work/first ]]
expect_fail "answered 404" scripts/site-previous.sh --url "$url" --out "$work/no-first"
[[ ! -e $work/no-first ]]

echo "scripts/site-previous.test.sh: a 500 is not a first deploy"
serve status-500
expect_fail "answered 500" scripts/site-previous.sh --allow-first --url "$url" --out "$work/down"
expect_fail "answered 500" scripts/site-previous.sh --url "$url" --out "$work/down-plain"
stop_server

echo "scripts/site-previous.test.sh: network error"
expect_fail "could not fetch" scripts/site-previous.sh --url "$url" --out "$work/offline"
echo "scripts/site-previous.test.sh: ok"
