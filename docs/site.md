# The site

`apps/site` is Get orbe at [orbefab.com/embedded](https://orbefab.com/embedded): how to install orbe and run it, the parts catalog, the Blocks, and the `@orbefab` registry itself. Every page is static and is built from one registry build of this repo, so a part or Block added here reaches the site at the next build with no change to the site. The header, footer and theme match orbefab.com.

| Path | What it shows |
| --- | --- |
| `/embedded/` | Get orbe: what it is, the three points, and the four steps. With install lines off, each step says the CLI is coming. |
| `/embedded/parts/` | Every part as a card: its title, whether it matches a maker's number, its id, and how many Blocks place it (or which board it sits on). A search over the fields `orbe search` reads, in its order. `?q=` holds the query. |
| `/embedded/parts/<id>/` | The part in 3D beside its install line and a short glance, the Blocks whose scenes place it and the boards it sits inside (a part can be both), its pins, params, ratings with their sources, what is not simulated, what it is built from, its docs, where to buy it with the date each link was checked, and a closed license box with every file, its size and its license. A part that names a maker says Orbefab is not affiliated with or endorsed by that maker. |
| `/embedded/blocks/` | Every Block, with the board's name and how many parts its scene places. |
| `/embedded/blocks/<id>/` | The Block live in 3D, or its sketch, scene and tests, on a Preview / Code toggle. The install line sits beside the toggle. Below: the README, the parts the scene places (a board's own parts folded under the board), and a closed license box with every file and its license. The part count is the scene's, the same number as the card. The test count is the number of `tests/*.test` files, which is what `orbe test` reports. |
| `/embedded/blocks/<id>/embed/` | That Block's player only: the stage, its caption and Hold to press. No header or footer. `noindex`. The home frames `nano-button-led` from here. |
| `/embedded/r/` | The registry `orbe registry build` writes, so `@orbefab`'s template is `https://orbefab.com/embedded/r/{name}.json`. |
| `/embedded/sources/` | The AVR runtime's source archive, which every Block's `firmware/NOTICE` names. |
| `/embedded/install.sh` | The install script, copied from `scripts/install.sh` when install lines are on. |

Sparse parts render as they are. A missing summary, tag, doc or seller shows as missing, never made up.

## How it builds

```sh
pnpm install --frozen-lockfile
scripts/site.sh --first-deploy
```

`scripts/site.sh` fetches the AVR runtime's source into the blob cache, builds `orbe`, writes this repo's registry with `orbe registry build . .scratch/site/registry`, builds orbe-wasm with `scripts/wasm.sh .scratch/site/wasm`, then runs `vite build` in `apps/site`. It needs what [wasm.md](wasm.md#build) needs, Node 24 and pnpm, and installs nothing. It requires either `--previous <dir>` or `--first-deploy`. `--previous <dir>` is the registry the live site serves, and the build keeps every version that registry served, as [registry.md](registry.md#versions) says. `scripts/site-previous.sh` downloads that registry. `--first-deploy` is the first deploy, which has no earlier registry.

The site is a [TanStack Start](https://tanstack.com/start/latest) app, set up as the website is: file routes in `src/routes`, `@cloudflare/vite-plugin`, Tailwind 4 and `@vitejs/plugin-react`.

- **Inputs.** `build/inputs.ts` is a Vite plugin. It runs `scripts/brand.sh check` and stops on a brand copy that differs from its list. It reads the registry, orbe-wasm and the runtime source from `.scratch/site`. It then writes `.scratch/site/public`, the folder Vite serves at `/embedded/` and copies into the build: the brand's public files, the registry at `r/`, the package at `orbe-wasm/`, Syne's licence beside the font, the runtime source at `sources/`, each part's `view.json`, `_headers`, and, when install lines are on, `install.sh`. The catalog reaches the server side as the module `virtual:orbe-site`, and only `src/site.server.ts` imports it.
- **Pages.** Vite's `base` is `/embedded/`, and Start makes that the router's basepath, so a route file such as `src/routes/parts/$id.tsx` answers `/embedded/parts/<id>/`. Each route's loader is a server function in `src/data.ts` that returns what the page shows. Start prerenders every page at build time: it starts from `/embedded/`, the 404 page and each Block's embed route (nothing links to an embed), and follows each link it finds (`prerender.crawlLinks`). It renders in a Worker built from Start's server entry, and only the build runs that Worker. A loader's result is in the page's HTML, and the browser hydrates the page from it. Pages link to each other with plain anchors, so each navigation loads the next page's HTML and the browser never calls a server function. A click on a same-page hash, including a second click on the hash the page already has, stays on that page: the router keeps the prerendered loader data (`defaultStaleTime` is infinite), so that click does not call a server function either. `scripts/check-site.sh` fails when a page and the registry disagree on how many parts and Blocks there are.
- **A part's 3D.** The build runs orbe-wasm's `partScene` in Node on each part's files and those of every part it is built from, and writes the scene message to `/embedded/parts/<id>/view.json`, with each model pointed at its file in the registry. The part page draws it with `packages/viewer`, as `orbe show` draws a scene, with no Worker and no WebAssembly. The parts list shows no images.
- **A Block.** The player runs `packages/viewer`'s `workerSource` on orbe-wasm's `worker.js` from `/embedded/orbe-wasm/`, and `loadBlock` reads the Block from `/embedded/r/` and checks every hash, as [viewer.md](viewer.md#run-a-block-in-the-page) says. It plays as soon as the Block opens. Showing another Block, or leaving the page, closes the source, which terminates the Worker and revokes its object URLs.
- **three.js** and the viewer are a chunk of their own, `src/components/live.tsx`, which only a page with a 3D view loads, once its text has drawn.
- **The brand.** [`packages/brand`](../packages/brand/README.md) is the website's brand, copied unchanged. Its `website/public/` files are served at `/embedded/`, and its `styles.css` holds the Tailwind 4 tokens.

### Install lines

Until the repo and the binaries are public, an install command would fail, so the site shows none. `ORBE_SITE_INSTALL=1 scripts/site.sh --first-deploy` shows them: `orbe add <id>` on each part and Block page, each with a Copy button, and on Get orbe the install line, `orbe skills get core`, `orbe add nano-led-fade` with that Block's `scene.jsonc`, `orbe check scene.jsonc` and `orbe test`. That build also serves `scripts/install.sh` at `/embedded/install.sh`. A build without the flag does not, and each of those steps says the CLI is coming. The footer links the repo either way. A Block's README shows either way. The paragraph that says `orbe add <block>` shows only with the flag, and without the flag that paragraph is left out. The build defines the flag as a literal, `__ORBE_SITE_INSTALL__`, so a build without it holds none of the install text, in its pages or in its scripts.

The flag gates the site's own text and links. The registry's files are data, served as `orbe registry build` wrote them, so a Block's `README.md` and `block.jsonc` still say `orbe add`, and its `firmware/NOTICE` names the repo. `scripts/check-site.sh` checks every file under `dist/client` except `r/`, `orbe-wasm/` and `sources/`.

### The runtime source

Each Block's firmware image links the Arduino AVR core 1.8.6, which is LGPL-2.1-or-later, so publishing the image means offering that source. The site serves the exact archive beside the firmware, at `https://orbefab.com/embedded/sources/avr-1.8.6.tar.bz2`, and every `firmware/NOTICE` names that URL. `packages/engine/scripts/sketches.sh` pins the archive by URL and SHA-256. `scripts/site.sh` fetches it once into the blob cache, `$ORBE_BLOBS`, else `$XDG_CACHE_HOME/orbe/blobs`, else `~/.cache/orbe/blobs`, as a file named by its SHA-256. It refuses a download with another hash. Offline with an empty cache, it stops and says where to put the archive. The repo does not hold the archive.

## Look at it

```sh
cd apps/site
pnpm dev                                  # http://localhost:5173/embedded/, each page rendered on request
pnpm preview                              # the build, as it deploys
pnpm exec wrangler dev --local            # the same build, under wrangler
```

`pnpm dev` reads the inputs `scripts/site.sh` left in `.scratch/site`. `vite preview` and `wrangler dev` serve the deploy config the build wrote, `dist/client/wrangler.json`, with Cloudflare's own asset handling and `_headers`. Neither talks to a Cloudflare service.

## Check it

`scripts/check-site.sh` runs the type check, `pnpm --filter @orbe/site test` (the circuit readers and the value formatter) and the brand check. It builds the site with `ORBE_SITE_INSTALL=1` and fails unless every part and Block in the registry has a page with its `orbe add` line, each Block also has an embed page, the Get orbe page shows the install script line and links the repo, and `/embedded/install.sh` is `scripts/install.sh`. Playwright then drives that build on `vite preview` (`apps/site/e2e/install.spec.ts`): every part page and every Block page fits a phone width with the install line on screen, Get orbe shows the five commands and the scene, and Copy says when the write worked and when it did not. It builds the site again without the flag. That build fails the check if it serves `install.sh`, or if a file of the site's own shows `orbe add`, `cargo install` or the install script URL, or if the footer does not link the repo, or if it does not serve the runtime source that sketches.sh pins and every Block's NOTICE names. Then `@playwright/test` (`apps/site/e2e/site.spec.ts`) drives that build in headless Chromium, once on `vite preview` and once on `wrangler dev --local`:

- Get orbe says what orbe is, shows the three points and four steps, and plays no Block. With install lines off, each step says the CLI is coming. The page asks no server for data, loads every file from under `/embedded/`, and draws the wordmark in Syne from `/embedded/fonts/`. The header and footer are the ones shared with orbefab.com;
- the parts search lists every part, and for ten queries shows the parts `orbe search --json` gives, in its order;
- led-5mm's page draws its model, lists its five files with their licenses inside License and files, serves a download whose SHA-256 is the record's, lists the Blocks that place it, and lists DigiKey with its checked date and the not-affiliated line;
- nano-board's page draws the board with its children and lists the Blocks that place it. The Uno is titled as a compatible board, shows `MIT OR Apache-2.0` on its `part.jsonc`, `model.py` and GLB, and links the clone schematic it is drawn from;
- every Block's page plays it in the Worker. A button lights `@led1` while it is held, the fade's LED changes brightness as it plays, and the light comparator's slider lights `@led1` in the dark and turns it off in the light. Preview and Code switch the view. The Code view lists the sketch, the scene and every file under `tests/`. nano-button-led's files show their own licenses: the image and `LICENSE-runtime` LGPL-2.1-or-later, `LICENSE-libc` BSD-3-Clause, the scene and the sketch MIT OR Apache-2.0. A card's part count is the number of parts in that Block's `scene.jsonc`. A same-page `#` link, clicked again or opened already on that hash, stays on the page and calls no server function;
- an unknown path answers 404 with the site's page and logs no page error, and `/embeddedfoo` is not the site's;
- a registry file answers 304 to its own ETag, a missing one answers 404 with no immutable cache, a hashed build file keeps for a year, and a record's `@` path answers 307.

It prints each page's requests and bytes, uncompressed and gzipped, and the time to the first Block frame. It finds Chromium as `check-viewer-e2e.sh` does. `--renders <dir>` saves screenshots. `scripts/check.sh web` runs it, then `scripts/check-engines.sh`, which loads this build's Get orbe page, parts page and Blocks page in headless Firefox and headless WebKit on `vite preview` only. WebKit draws a Block page's Block. Headless Firefox shows the WebGL error described in [viewer.md](viewer.md#run-it).

## Hosting

`apps/site/wrangler.jsonc` is a Worker with no code. The build writes its deploy config, `dist/client/wrangler.json`, which serves `dist/client` as Workers static assets mounted at `/embedded` by `assets.base_path` ([Serving a subdirectory](https://developers.cloudflare.com/workers/static-assets/routing/advanced/serving-a-subdirectory/)). It runs on two routes, `orbefab.com/embedded` and `orbefab.com/embedded/*`. A pattern that ends in `*` matches every suffix, so `orbefab.com/embedded*` would also take `/embeddedfoo` ([Routes](https://developers.cloudflare.com/workers/configuration/routing/routes/)). The website's Worker holds `orbefab.com` as a Custom Domain, and a route takes precedence over a Custom Domain on the paths it matches, so this Worker answers `/embedded` and every path under it, and the website answers the rest.

- `_headers` caches `/embedded/assets/*` for a year, since their names are their hashes, and `/embedded/fonts/*` for 30 days. A rule in `_headers` applies to every response at a matching path, a 404 included ([Headers](https://developers.cloudflare.com/workers/static-assets/headers/)), and nothing in `_headers` or `not_found_handling` tells a hit from a miss. So `/embedded/r/files/*` keeps the default, `public, max-age=0, must-revalidate`, and a browser revalidates each file against its ETag, which answers 304 while it is unchanged. A registry file that is missing for a moment, during a deploy, is never cached. Every file gets `X-Content-Type-Options: nosniff`, and the registry gets `Access-Control-Allow-Origin: *`, so any page can read it.
- Workers static assets answer a path that holds `@` with a 307 to the same path with `%40`. A registry record such as `led-5mm@0.1.0.json` therefore costs one redirect, which browsers and `loadBlock` follow.
- A path under `/embedded` that holds no file gets `404.html`, the site's own page, with a 404.
- A build is about 430 files and 16 MB, 7 MB of it the runtime source. The registry stores each file once by its hash, and nothing beyond that is optimised.

## Deploy

A push to `main` runs [`.github/workflows/site.yml`](../.github/workflows/site.yml) on GitHub's `ubuntu-24.04` runner. The workflow also runs when someone starts it by hand. It does not run on a pull request, so a pull request never receives the deploy secrets. The hand run has one input, `deploy`, which defaults to false.

The push starts a job only when the repository variable `SITE_DEPLOY` is `on`. Until then the push skips the job and spends nothing. A hand run always builds and checks. It deploys only when `SITE_DEPLOY` is `on` and `deploy` is true. Deploys share one concurrency group and do not cancel each other, so two deploys never overlap.

`scripts/site-previous.sh` downloads `https://orbefab.com/embedded/r/` into the directory passed as `--previous`. It starts at `index.json` and fetches every record and every `files/<sha256>` that record lists. It writes a path only when a record's URL is `<name>@<version>.json` and a file's URL is `files/` plus 64 lowercase hex. `orbe registry build --previous` verifies everything that download contains. A network error, a status other than 200, or a redirect that leaves that directory fails the job.

The first deploy has no earlier registry. Set the repository variable `SITE_FIRST_DEPLOY` to `1` for that run. The workflow then passes `--first-deploy` only when `index.json` answers 404. A 404 with the variable unset fails. A 200 with the variable set also fails, so the variable is unset after the first deploy. The website has to answer that URL with 404 until the Worker exists. Any other status fails the job, and the manual path below is the fallback.

Set the repository variable `ORBE_SITE_INSTALL` to `1` to deploy with install lines. `scripts/check-site.sh` builds the site both ways and leaves the build with the lines off. When the variable is `1`, the workflow runs `scripts/site.sh` once more with `ORBE_SITE_INSTALL=1` and uploads that build.

The deploy step reads two secrets.

| Secret | What it is |
| --- | --- |
| `CLOUDFLARE_API_TOKEN` | An account API token for the account in `CLOUDFLARE_ACCOUNT_ID`. |
| `CLOUDFLARE_ACCOUNT_ID` | The account that owns the Worker `orbefab-embedded`. |

The token deploys that Worker and its two routes on the `orbefab.com` zone. That is Workers Editor on the Worker `orbefab-embedded`, and zone Workers Routes Write on the zone `orbefab.com`. The first deploy creates the Worker, and a token cannot be scoped to a Worker that does not exist yet, so that first token is Workers Admin on the account. After the Worker exists, replace the Admin with Editor on `orbefab-embedded`. The same access under the older token names is account Workers Scripts Edit and zone Workers Routes Edit on `orbefab.com`. The account permission is not limited to this Worker. [Workers roles](https://developers.cloudflare.com/workers/authorization/workers/) names the current roles.

`wrangler deploy` runs in `apps/site`. It follows `.wrangler/deploy/config.json`, which the build wrote, to `dist/client/wrangler.json`, and uploads `dist/client` as assets and no Worker code. Do not pass `--config wrangler.jsonc`, which skips the built config. `scripts/site-deployed.sh` then requests the live site and fails the job unless:

- `https://orbefab.com/embedded/` shows Get orbe, and `https://orbefab.com/` and `https://orbefab.com/embeddedfoo` still go to the website.
- `https://orbefab.com/embedded/r/index.json` is this build's index, and `https://orbefab.com/embedded/parts/led-5mm/` downloads its files.
- `https://orbefab.com/embedded/sources/avr-1.8.6.tar.bz2` downloads, with the SHA-256 every Block NOTICE names.
- `https://orbefab.com/embedded/assets/<file>` has the year-long `Cache-Control`, and `https://orbefab.com/embedded/r/files/<sha256>` has an `ETag`.
- `https://orbefab.com/embedded/no-such-page` answers 404 with this site's page, not the website's.

With install lines on, Get orbe shows the install command and `/embedded/install.sh` is `scripts/install.sh`. With them off, Get orbe says the CLI is coming and that URL answers 404.

The same steps, run by hand from a clean checkout of `main`, are the fallback when the workflow cannot:

```sh
git status --short                 # nothing
pnpm install --frozen-lockfile
scripts/site-previous.sh --url https://orbefab.com/embedded/r/ --out .scratch/site/previous
scripts/check-site.sh --previous .scratch/site/previous
cd apps/site
pnpm exec wrangler deploy          # reads dist/client/wrangler.json, which the build wrote
```

The first deploy uses `scripts/check-site.sh --first-deploy` and skips `scripts/site-previous.sh`. A directory saved from the last deploy's `dist/client/r` can stand in for the download. `ORBE_SITE_INSTALL=1 scripts/site.sh --previous .scratch/site/previous` rebuilds with install lines before `wrangler deploy` when those lines should show. `scripts/site-deployed.sh` runs the live checks.
