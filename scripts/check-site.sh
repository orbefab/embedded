#!/usr/bin/env bash
# Checks the site. It runs the type check, the circuit unit tests and the brand check, then a
# build with ORBE_SITE_INSTALL=1. scripts/site-install.sh checks that build: every part and
# Block page shows its orbe add line, and Get orbe shows the install command, the repo link
# and install.sh. The site workflow runs the same script on the build it uploads. Playwright
# drives that build once, on `vite preview`, for the checks that need the install line. Then
# it runs the build as it deploys, whose own files must show no install line.
# The footer links the repo either way. The build must serve the AVR runtime source every
# Block's NOTICE names.
# Playwright then drives that build in headless Chromium: a short `@vite-smoke` subset on
# `vite preview` (home, one part page, one Block page) and the full site suite on
# `wrangler dev --local`.
#
# The flag gates the site's own text and links: the pages, the build's scripts and styles, the
# brand files and the part views. The registry under r/, orbe-wasm and the runtime source are data
# the site serves unchanged, so a Block's README or block.jsonc still says `orbe add`.
#
# Needs what scripts/site.sh needs, and a Chromium found as check-viewer-e2e.sh finds it. It
# installs nothing. `--renders <dir>` saves screenshots there.
# `--servers vite` runs the install-line checks (`install.spec.ts`) and the `@vite-smoke` subset
# on `vite preview`. `--servers wrangler` skips the install-line build and drives the full suite
# on `wrangler dev --local`. `--shard N/M` passes that
# shard to the suite. `--no-build` uses the site already at apps/site/dist and does not run the
# deploy build. The install-line build still runs unless `--servers wrangler`. The build
# carries dist/wrangler-deploy.json, the pointer `vite preview` and wrangler follow. Before
# either server starts, `scripts/site-deploy-pointer.sh` puts it back at
# apps/site/.wrangler/deploy/config.json and checks its built config. Set ORBE to
# the orbe binary when this machine has not built it. `--no-playwright` stops after the file checks.
# The arguments can be in any order. With none of `--servers` or `--shard`, the check runs both
# hosts, as `scripts/check.sh web` does. An unknown argument is an error.
set -euo pipefail
cd "$(dirname "$0")/.."
renders=""
saw_renders=0
servers=both
shard=""
no_build=0
no_playwright=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --renders)
      if [[ $# -lt 2 || -z ${2:-} || ${2:-} == --* ]]; then
        echo "check-site: --renders needs a directory" >&2
        exit 2
      fi
      renders=$2
      saw_renders=1
      shift 2
      ;;
    --servers)
      case ${2:-} in
        both | vite | wrangler) servers=$2 ;;
        *)
          echo "check-site: --servers needs both, vite or wrangler" >&2
          exit 2
          ;;
      esac
      shift 2
      ;;
    --shard)
      if [[ $# -lt 2 || ! ${2:-} =~ ^[1-9][0-9]*/[1-9][0-9]*$ ]]; then
        echo "check-site: --shard needs N/M, such as 1/2" >&2
        exit 2
      fi
      shard_n=${2%/*}
      shard_m=${2#*/}
      if ((shard_n > shard_m)); then
        echo "check-site: --shard $2 is out of range" >&2
        exit 2
      fi
      shard=$2
      shift 2
      ;;
    --no-build)
      no_build=1
      shift
      ;;
    --no-playwright)
      no_playwright=1
      shift
      ;;
    *)
      echo "check-site: unknown argument $1" >&2
      exit 2
      ;;
  esac
done
if [[ $saw_renders -eq 1 ]]; then
  export ORBE_RENDERS=$renders
fi
scripts/need-node-deps.sh scripts/check-site.sh
# shellcheck source=scripts/check-lock.sh
source scripts/check-lock.sh
check_lock "scripts/check-site.sh" "scripts/check-site.sh $*"
fail() {
  echo "check-site: $1" >&2
  exit 1
}
dist=apps/site/dist/client
if [[ $no_build -eq 1 && ! -f $dist/index.html ]]; then
  fail "no site build at $dist. Run scripts/site.sh first."
fi
if [[ $no_build -eq 0 ]]; then
  pnpm --filter @orbe/site typecheck
  pnpm --filter @orbe/site test
  scripts/brand.sh check
fi
own() { grep -rlE "$1" "$dist" --exclude-dir=r --exclude-dir=orbe-wasm --exclude-dir=sources || true; }

# shellcheck source=scripts/site-port.sh
source scripts/site-port.sh

deploy_saved=""
restore_deploy() {
  [[ -n ${deploy_saved} && -d ${deploy_saved} ]] || return 0
  rm -rf apps/site/dist
  mkdir -p apps/site/dist
  cp -a "${deploy_saved}/." apps/site/dist/
  rm -rf "${deploy_saved}"
  deploy_saved=""
}
trap restore_deploy EXIT

if [[ $servers != wrangler ]]; then
  if [[ $no_build -eq 1 ]]; then
    deploy_saved=$(mktemp -d)
    cp -a apps/site/dist/. "$deploy_saved/"
  fi
  ORBE_SITE_INSTALL=1 scripts/site.sh >/dev/null
  scripts/site-install.sh

  export ORBE_E2E=site
  export ORBE_SITE_INSTALL_E2E=1
  export ORBE_SITE_SERVERS=vite
  echo "check-site: install-on browser checks"
  with_fresh_ports ORBE_SITE_VITE_PORT -- pnpm exec playwright test apps/site/e2e/install.spec.ts 9>&-
  unset ORBE_SITE_INSTALL_E2E ORBE_SITE_SERVERS
  restore_deploy
fi

if [[ $no_build -eq 0 ]]; then
  scripts/site.sh
fi
[[ ! -e $dist/install.sh ]] || fail "install.sh is served with ORBE_SITE_INSTALL unset"
shown=$(own 'orbe add|cargo install|orbefab\.com/orbe/install\.sh')
[[ -z $shown ]] || fail "these files of the site's own show an install line with ORBE_SITE_INSTALL unset: $shown"
grep -q 'href="https://github.com/orbefab/orbe"' "$dist/index.html" ||
  fail "the footer does not link the repo"
grep -q 'name="robots" content="noindex"' "$dist/blocks/nano-button-led/embed/index.html" ||
  fail "the embed page is not noindex"
echo "check-site: no file of the site's own shows an install line with ORBE_SITE_INSTALL unset, and the footer links the repo"

pin() { sed -n "s/^$1=//p" packages/engine/scripts/sketches.sh; }
archive=$(pin runtime_url)
archive=${archive##*/}
[[ $(sha256sum "$dist/sources/$archive" | cut -d' ' -f1) == "$(pin runtime_sha256)" ]] ||
  fail "$dist/sources/$archive is not the runtime source sketches.sh pins"
# shellcheck disable=SC2016 # javascript
node -e '
const [r, want] = process.argv.slice(1);
const fs = require("node:fs");
const index = JSON.parse(fs.readFileSync(`${r}/index.json`, "utf8"));
for (const row of index.items.filter((row) => row.type === "block")) {
  const record = JSON.parse(fs.readFileSync(`${r}/${row.url}`, "utf8"));
  if (record.board === "esp32s3-devkitc-1") continue;
  const notice = record.files.find((file) => file.path === "firmware/NOTICE");
  if (notice === undefined || !fs.readFileSync(`${r}/files/${notice.sha256}`, "utf8").includes(want)) {
    console.error(`check-site: ${row.name}@${row.version} has no firmware/NOTICE that names ${want}`);
    process.exit(1);
  }
}' "$dist/r" "https://orbefab.com/embedded/sources/$archive"
echo "check-site: the build serves sources/$archive, the runtime source each Block NOTICE names"
[[ -f apps/site/dist/wrangler-deploy.json ]] ||
  fail "the build has no wrangler deploy pointer at apps/site/dist/wrangler-deploy.json"

if [[ $no_playwright -eq 1 ]]; then
  exit 0
fi

scripts/site-deploy-pointer.sh check-site

if [[ $no_build -eq 0 || -z ${ORBE:-} ]]; then
  target=$(cargo metadata --format-version 1 --no-deps |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
  export ORBE=$target/debug/orbe
fi
[[ -x ${ORBE} ]] || fail "no orbe at ${ORBE}. Run \`cargo build -p orbe --locked\` first."
# `orbe search` reads the registry this build wrote, the same index the parts page searches.
registry=$(cd "$dist/r" && pwd)
export ORBE_REGISTRY="file://${registry}/{name}.json"
export ORBE_E2E=site
export WRANGLER_SEND_METRICS=false
if [[ $servers != both ]]; then
  export ORBE_SITE_SERVERS=$servers
fi
if [[ -n $shard ]]; then
  with_fresh_ports ORBE_SITE_VITE_PORT ORBE_SITE_WRANGLER_PORT -- pnpm exec playwright test --shard="$shard" 9>&-
else
  with_fresh_ports ORBE_SITE_VITE_PORT ORBE_SITE_WRANGLER_PORT -- pnpm exec playwright test 9>&-
fi
