# The site

`apps/site` is Get orbe at [orbefab.com/orbe](https://orbefab.com/orbe): how to install orbe and run it, the parts catalog, the Blocks, and the `@orbefab` registry itself. Every page is static and is built from the registry the release tags `v0.2.0` and later publish. A part or Block reaches the site when a release tag contains it, and an older tag's record stays. The header, footer and theme match orbefab.com.

| Path | What it shows |
| --- | --- |
| `/orbe/` | Get orbe: what it is, the three points, and the five steps. With install lines off, each step says the CLI is coming. |
| `/orbe/parts/` | Every part as a card: its title, and where it is used when a Block or a board carries it. A search over the fields `orbe search` reads, in its order. `?q=` holds the query. |
| `/orbe/parts/<id>/` | The part in 3D beside its install line and a short glance of its pins and where it is used. Below, when each has something to show: the Blocks whose scenes place it, the boards it sits inside, its pins, its settings, its ratings, what is not simulated, what it is built from, its docs, and where to buy it with the date each link was checked. A rating's source is a Datasheet link. A board's pins are names. A breadboard says that holes in a strip are connected. A closed license box lists every file and its size, and the license expression. |
| `/orbe/blocks/` | Every Block. Each card names the board and how many parts its scene places. |
| `/orbe/blocks/<id>/` | The Block live in 3D, or its sketch, scene and tests, on a Preview / Code toggle. The install line sits beside the toggle. The line under the stage is the Block's summary. Below: the README, the parts the scene places, and a closed license box with the license expression and every file. The part count is the scene's, the same number as the card. Hold to press is the first placed part with a press input. The test count is the number of `tests/*.test` files, which is what `orbe test` reports. |
| `/orbe/blocks/<id>/embed/` | That Block's player only: the stage, the Block's summary and Hold to press. A Block with no summary has no line under the stage. No header or footer. `noindex`. The home frames `nano-button-led` from here. |
| `/orbe/r/` | The registry the release tags publish, so `@orbefab`'s template is `https://orbefab.com/orbe/r/{name}.json`. |
| `/orbe/sources/` | The AVR runtime's source archive. Every Block's `firmware/NOTICE` names `https://orbefab.com/embedded/sources/`, and that path redirects here. |
| `/orbe/install.sh` | The install script, copied from `scripts/install.sh` when install lines are on. |

A section with nothing in it is left out. A missing summary is left out, never made up.

## How it builds

```sh
pnpm install --frozen-lockfile
scripts/site.sh
```

`scripts/site.sh` fetches the AVR runtime's source into the blob cache, builds `orbe`, writes the registry with `scripts/registry.sh` from the tags `v0.2.0` and later into `.scratch/site/registry`, builds orbe-wasm with `scripts/wasm.sh .scratch/site/wasm`, then runs `vite build` in `apps/site`. It needs what [wasm.md](wasm.md#build) needs, Node 24 and pnpm, and installs nothing. An older tag's record stays beside a newer one. [registry.md](registry.md#versions) says how a record, once served, stays fetchable.

The site is a [TanStack Start](https://tanstack.com/start/latest) app, set up as the website is: file routes in `src/routes`, `@cloudflare/vite-plugin`, Tailwind 4 and `@vitejs/plugin-react`.

- **Inputs.** `build/inputs.ts` is a Vite plugin. It runs `scripts/brand.sh check` and stops on a brand copy that differs from its list. It reads the registry, orbe-wasm and the runtime source from `.scratch/site`. It then writes `.scratch/site/public`, the folder Vite serves at `/orbe/` and copies into the build: the brand's public files, the registry at `r/`, the package at `orbe-wasm/`, Syne's licence beside the font, the runtime source at `sources/`, each part's `view.json`, `_headers`, and, when install lines are on, `install.sh`. The catalog reaches the server side as the module `virtual:orbe-site`, and only `src/site.server.ts` imports it.
- **Pages.** Vite's `base` is `/orbe/`, and Start makes that the router's basepath, so a route file such as `src/routes/parts/$id.tsx` answers `/orbe/parts/<id>/`. Each route's loader is a server function in `src/data.ts` that returns what the page shows. Start prerenders every page at build time: it starts from `/orbe/`, the 404 page, each embed route of a Block in the registry (nothing links to an embed) and each part whose index row sets `listed` to false (the parts list does not link it), and follows each link it finds (`prerender.crawlLinks`). A part or Block that is only in the repo has no page yet. It renders in a Worker built from Start's server entry, and only the build runs that Worker. A loader's result is in the page's HTML, and the browser hydrates the page from it. Pages link to each other with plain anchors, so each navigation loads the next page's HTML and the browser never calls a server function. A click on a same-page hash, including a second click on the hash the page already has, stays on that page: the router keeps the prerendered loader data (`defaultStaleTime` is infinite), so that click does not call a server function either. `scripts/check-site.sh` fails when a page and the registry disagree on how many parts and Blocks there are.
- **A part's 3D.** The build runs orbe-wasm's `partScene` in Node on each part's files and those of every part it is built from, and writes the scene message to `/orbe/parts/<id>/view.json`, with each model pointed at its file in the registry. The part page draws it with `packages/viewer`, as `orbe show` draws a scene, with no Worker and no WebAssembly. The parts list shows no images.
- **A Block.** The player runs `packages/viewer`'s `workerSource` on orbe-wasm's `worker.js` from `/orbe/orbe-wasm/`, and `loadBlock` reads the Block from `/orbe/r/` and checks every hash, as [viewer.md](viewer.md#run-a-block-in-the-page) says. It plays as soon as the Block opens. Showing another Block, or leaving the page, closes the source, which terminates the Worker and revokes its object URLs. The build runs orbe-wasm's `sceneParts` for each Block. That list is the card's part count, the parts on the page, and Hold to press, which names the first placed part with a press input. The line under the stage is the Block's summary. A Block added to the repo needs no change to the site.
- **three.js** and the viewer are a chunk of their own, `src/components/live.tsx`, which only a page with a 3D view loads, once its text has drawn.
- **The brand.** [`packages/brand`](../packages/brand/README.md) is the website's brand, copied unchanged. Its `website/public/` files are served at `/orbe/`, and its `styles.css` holds the Tailwind 4 tokens.

### Install lines

Until the repo and the binaries are public, an install command would fail, so the site shows none. `ORBE_SITE_INSTALL=1 scripts/site.sh` shows them: `orbe add <id>` on each part and Block page, each with a Copy button, and on Get orbe the install line, `orbe skills get core`, `orbe add nano-button-led` with that Block's `scene.jsonc`, `orbe check scene.jsonc`, `orbe test`, `orbe open --headed`, `orbe play` and `orbe close`. That build also serves `scripts/install.sh` at `/orbe/install.sh`. A build without the flag does not, and each of those steps says the CLI is coming. The footer links the repo either way. A Block's README shows either way. The paragraph that says `orbe add <block>` shows only with the flag, and without the flag that paragraph is left out. The build defines the flag as a literal, `__ORBE_SITE_INSTALL__`, so a build without it holds none of the install text, in its pages or in its scripts.

The flag gates the site's own text and links. The registry's files are data, served as `orbe registry build` wrote them, so a Block's `README.md` and `block.jsonc` still say `orbe add`, and its `firmware/NOTICE` names the repo. `scripts/check-site.sh` checks every file under `dist/client` except `r/`, `orbe-wasm/` and `sources/`.

### The runtime source

Each Block's firmware image links the Arduino AVR core 1.8.6, which is LGPL-2.1-or-later, so publishing the image means offering that source. The site serves the exact archive beside the firmware, at `https://orbefab.com/orbe/sources/avr-1.8.6.tar.bz2`. Every `firmware/NOTICE` names `https://orbefab.com/embedded/sources/avr-1.8.6.tar.bz2`, and that URL redirects here. `packages/engine/scripts/sketches.sh` pins the archive by URL and SHA-256. `scripts/site.sh` fetches it once into the blob cache, `$ORBE_BLOBS`, else `$XDG_CACHE_HOME/orbe/blobs`, else `~/.cache/orbe/blobs`, as a file named by its SHA-256. It refuses a download with another hash. Offline with an empty cache, it stops and says where to put the archive. The repo does not hold the archive.

## Look at it

```sh
cd apps/site
pnpm dev                                  # http://localhost:5173/orbe/, each page rendered on request
pnpm preview                              # the build, as it deploys
pnpm exec wrangler dev --local            # the same build, under wrangler
```

`pnpm dev` reads the inputs `scripts/site.sh` left in `.scratch/site`. `vite preview` and `wrangler dev` serve the deploy config the build wrote, `dist/orbefab_embedded/wrangler.json`, with Cloudflare's own asset handling, `_headers` and the `/embedded` redirect. Neither talks to a Cloudflare service.

## Check it

`scripts/check-site.sh` runs the type check, `pnpm --filter @orbe/site test` (the circuit readers and the value formatter) and the brand check. It builds the site with `ORBE_SITE_INSTALL=1` and fails unless every part and Block in the registry has a page, each Block also has an embed page, and `scripts/site-install.sh` passes: every part and Block page shows its `orbe add` line, the Get orbe page shows the install command and links the repo, and `/orbe/install.sh` is `scripts/install.sh`. Playwright then drives that build on `vite preview` (`apps/site/e2e/install.spec.ts`): every part page and every Block page fits a phone width with the install line on screen, Get orbe shows the eight commands and the scene, and Copy says when the write worked and when it did not. It builds the site again without the flag. That build fails the check if it serves `install.sh`, or if a file of the site's own shows `orbe add`, `cargo install` or the install script URL, or if the footer does not link the repo, or if it does not serve the runtime source that sketches.sh pins and every Block's NOTICE names. Then `@playwright/test` (`apps/site/e2e`, including `site.spec.ts`) drives that build in headless Chromium, once on `vite preview` and once on `wrangler dev --local`:

- Get orbe says what orbe is, shows the three points and five steps, and plays no Block. With install lines off, each step says the CLI is coming. The page asks no server for data, loads every file from under `/orbe/`, and draws the wordmark in Syne from `/orbe/fonts/`. The header and footer are the ones shared with orbefab.com;
- the parts search lists every part whose index row does not set `listed` to false, and for ten queries shows the parts `orbe search --json` gives, in its order;
- led-5mm's page draws its model, lists its three files (`part.jsonc`, the anchors and the GLB) inside License and files under the shared license expression, serves a download whose SHA-256 is the record's, lists the Blocks that place it, and lists DigiKey with its checked date;
- nano-board's page draws the board with its children and lists the Blocks that place it. The Uno is titled as a compatible board, shows `MIT OR Apache-2.0` on the license box, lists `part.jsonc` and the GLB, and links the clone schematic it is drawn from;
- every Block the registry lists plays on its page. A button lights `@led1` while it is held, the fade's LED changes brightness as it plays, and the light comparator's slider lights `@led1` in the dark and turns it off in the light. A Block whose scene places a pixel stick shows a different colour on each pixel. The DevKitC-1 Block's button changes `@board.led` from red to green. The 7-segment Block lights a segment. A buzzer Block's simulated time moves. Preview and Code switch the view. The Code view lists the sketch, the scene and every file under `tests/`. nano-button-led's files share the Block's licence, `(MIT OR Apache-2.0) AND LGPL-2.1-or-later`, so the box has no per-file License column. It lists `firmware/NOTICE`, which states which file carries which licence, and it does not list `REUSE.toml`. A card's part count is the number of parts in that Block's `scene.jsonc`. A same-page `#` link, clicked again or opened already on that hash, stays on the page and calls no server function;
- an unknown path answers 404 with the site's page and logs no page error, and `/orbefoo` is not the site's. `/embedded`, `/embedded/`, `/embedded/r/<part>.json`, `/embedded/install.sh` and a page with a query answer 301 to the same path under `/orbe`, and the query stays. `/embeddedfoo` is not a redirect;
- a hashed registry file keeps for a year and answers 304 to its own ETag, a missing one answers 404 with no immutable cache, a hashed build file keeps for a year, and a record's `@` path answers 200 with that year-long cache.

It prints each page's requests and bytes, uncompressed and gzipped, and the time to the first Block frame. It finds Chromium as `check-viewer-e2e.sh` does. `--renders <dir>` saves screenshots. `--servers vite` or `--servers wrangler` drives the suite on that one host. `--servers wrangler` skips the install-line build, which needs `vite preview`. `--shard N/M` runs that Playwright shard of the suite. `--no-build` uses the site already at `apps/site/dist` and skips the deploy build. Before starting `vite preview` or `wrangler`, `scripts/site-deploy-pointer.sh` copies `dist/wrangler-deploy.json` back to `.wrangler/deploy/config.json`. That file is the pointer the build wrote, and both servers follow it to `dist/orbefab_embedded/wrangler.json`, whose `assets.directory` is `dist/client`. The source `wrangler.jsonc` names assets and no directory, so neither server starts without the pointer. The install-line build still runs unless the server is wrangler. Set `ORBE` to the orbe binary when this machine has not built it. `--no-playwright` stops after the file checks. With none of those flags the check runs both hosts. `scripts/check-engines.sh` restores the same pointer and loads this build's Get orbe page, parts page and Blocks page in headless Firefox and headless WebKit on `vite preview` only. WebKit draws a Block page's Block. Headless Firefox shows the WebGL error described in [viewer.md](viewer.md#run-it).

## Hosting

`apps/site/wrangler.jsonc` names the Worker `orbefab-embedded`. The name stays, because a new name would be a second Worker and this one would keep the routes. The build writes the deploy config, `dist/orbefab_embedded/wrangler.json`. Its `main` is `redirect.js`, bundled, and it serves `dist/client` as Workers static assets mounted at `/orbe` by `assets.base_path` ([Serving a subdirectory](https://developers.cloudflare.com/workers/static-assets/routing/advanced/serving-a-subdirectory/)). A request under `/orbe` is an asset, except `/orbe/r/*`, which runs the Worker first. A request for `/embedded` or a path under it is not an asset. The Worker answers 301 with the same path under `/orbe`, and the query stays. `/embeddedfoo` does not match. It runs on four routes: `orbefab.com/orbe`, `orbefab.com/orbe/*`, `orbefab.com/embedded` and `orbefab.com/embedded/*`. A pattern that ends in `*` matches every suffix, so `orbefab.com/orbe*` would also take `/orbefoo`, and the same for `/embedded` ([Routes](https://developers.cloudflare.com/workers/configuration/routing/routes/)). The website's Worker holds `orbefab.com` as a Custom Domain, and a route takes precedence over a Custom Domain on the paths it matches, so this Worker answers `/orbe` and `/embedded`, and the website answers the rest.

- `_headers` caches `/orbe/assets/*` for a year, since their names are their hashes, and `/orbe/fonts/*` for 30 days. A rule in `_headers` applies to every response at a matching path, a 404 included ([Headers](https://developers.cloudflare.com/workers/static-assets/headers/)), and nothing in `_headers` or `not_found_handling` tells a hit from a miss. So `_headers` does not cache `/orbe/r/`. The Worker sets `Cache-Control: public, max-age=31536000, immutable` on a 200 for `files/<sha256>` and for `<id>@<version>.json`. A 404 stays uncached, and so do `index.json` and the schemas, which change in place. A browser that sends the ETag still gets 304 while the file is unchanged. Every file gets `X-Content-Type-Options: nosniff`, and the registry gets `Access-Control-Allow-Origin: *`, so any page can read it.
- The Worker fetches a registry path that holds `@` as the same path with `%40`, which is the name of the asset. The response is that file, with no 307.
- A path under `/orbe` that holds no file gets `404.html`, the site's own page, with a 404.
- A build is about 430 files and 16 MB, 7 MB of it the runtime source. The registry stores each file once by its hash, and nothing beyond that is optimised.

## Deploy

A push to `main` runs [`.github/workflows/site.yml`](../.github/workflows/site.yml) on GitHub's `ubuntu-24.04` runner. The workflow also runs when someone starts it by hand. The hand run has one input, `deploy`, which defaults to false. The `site` environment, limited to the `main` branch, holds the two secrets. That is what keeps them from other branches and from pull requests.

The push starts a job only when the repository variable `SITE_DEPLOY` is `1`. Until then the push skips the job and spends nothing. A hand run works on `main` only, because the job uses the `site` environment, which allows only `main`. It always builds and checks, and deploys only when `SITE_DEPLOY` is `1` and `deploy` is true. Deploy runs on `main` share one concurrency group, and a build that does not deploy does not. A newer pending deploy replaces an older pending one, so the latest commit wins. A deploy that has started is left to finish.

Set the repository variable `ORBE_SITE_INSTALL` to `1` to deploy with install lines. `scripts/check-site.sh` builds the site both ways and leaves the build with the lines off. When the variable is `1`, the workflow runs `scripts/site.sh` once more with `ORBE_SITE_INSTALL=1` and `scripts/site-install.sh` checks that build before it is uploaded: every part and Block in the registry has a page and each Block an embed page, every part and Block page shows its `orbe add` line, Get orbe shows the install command and the repo link, and `install.sh` is served and equals `scripts/install.sh`.

The deploy step reads two secrets from the `site` environment.

| Secret | What it is |
| --- | --- |
| `CLOUDFLARE_API_TOKEN` | An account API token for the account in `CLOUDFLARE_ACCOUNT_ID`. |
| `CLOUDFLARE_ACCOUNT_ID` | The account that owns the Worker `orbefab-embedded`. |

The token is Workers Editor on the Worker `orbefab-embedded`, plus zone Workers Routes Edit on `orbefab.com`. `wrangler deploy` reads the zone's routes on every deploy, even when they do not change. `apps/site/wrangler.jsonc` names the zone by its ID, so the token needs no other zone permission to find it. Under the older token names, Workers Editor is account Workers Scripts Edit, which is not limited to this Worker. [Workers roles](https://developers.cloudflare.com/workers/authorization/workers/) names the current roles.

`wrangler deploy` runs in `apps/site`. It follows `.wrangler/deploy/config.json`, which the build wrote, to `dist/orbefab_embedded/wrangler.json`, and uploads `dist/client` as assets and the bundled `redirect.js` as the Worker. Do not pass `--config wrangler.jsonc`, which skips the built config. `scripts/site-deployed.sh` then requests the live site and fails the job unless:

- `https://orbefab.com/orbe/` shows Get orbe. `https://orbefab.com/` answers 200 with the website's title, which says Orbefab and not Get orbe. `https://orbefab.com/orbefoo` and `https://orbefab.com/embeddedfoo` answer 404.
- `https://orbefab.com/embedded`, `https://orbefab.com/embedded/`, `https://orbefab.com/embedded/r/led-5mm.json`, `https://orbefab.com/embedded/install.sh` and `https://orbefab.com/embedded/parts/?q=led` answer 301 to the same path under `/orbe`, and the query stays.
- `https://orbefab.com/orbe/r/index.json` is this build's index, and `https://orbefab.com/orbe/parts/led-5mm/` is that part's page and downloads every file of its record.
- `https://orbefab.com/orbe/sources/avr-1.8.6.tar.bz2` downloads, with the SHA-256 every AVR Block NOTICE names. The DevKitC-1 Block's NOTICE names the Arduino-ESP32 core instead. `https://orbefab.com/embedded/sources/avr-1.8.6.tar.bz2` redirects there.
- `https://orbefab.com/orbe/assets/<file>` has the year-long `Cache-Control`.
- `https://orbefab.com/orbe/no-such-page` answers 404 with this site's page, not the website's.

With install lines on, Get orbe shows the install command and `/orbe/install.sh` is `scripts/install.sh`. With them off, Get orbe says the CLI is coming and that URL answers 404. When the live check fails after a deploy, run `pnpm exec wrangler rollback` in `apps/site`, then fix forward.

The same steps, run by hand from a clean checkout of `main`, are the fallback when the workflow cannot.

```sh
git status --short                 # nothing
pnpm install --frozen-lockfile
scripts/check-site.sh
cd apps/site
pnpm exec wrangler deploy          # reads dist/orbefab_embedded/wrangler.json, which the build wrote
```

`ORBE_SITE_INSTALL=1 scripts/site.sh` rebuilds with install lines before `wrangler deploy` when those lines should show, and `scripts/site-install.sh` checks that build. `scripts/site-deployed.sh` runs the live checks.
