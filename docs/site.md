# The site

`apps/site` is the homepage at [orbefab.com/embedded](https://orbefab.com/embedded): what orbe is, with a Block playing live, the parts catalog, the Blocks, and the `@orbefab` registry itself. Every page is static and is built from one registry build of this repo, so a part or Block added here reaches the site at the next build with no change to the site.

| Path | What it shows |
| --- | --- |
| `/embedded/` | What orbe is, every Block live in 3D with a tab each, the Blocks and the boards. |
| `/embedded/parts/` | Every part, and a search over the fields `orbe search` reads, in its order. `?q=` holds the query. |
| `/embedded/parts/<id>/` | The part in 3D, its pins, params, ratings with their sources, what is not simulated, what it is built from and what uses it, every file with its size and licence as a download, its docs, where to buy it with the date each link was checked, and its versions. A part that names a maker says Orbefab is not affiliated with or endorsed by that maker. |
| `/embedded/blocks/` | Every Block. |
| `/embedded/blocks/<id>/` | The Block live in 3D, its parts at their versions, its scene, firmware source and tests, its licence and every file with its own licence. |
| `/embedded/r/` | The registry `orbe registry build` writes, so `@orbefab`'s template is `https://orbefab.com/embedded/r/{name}.json`. |
| `/embedded/sources/` | The AVR runtime's source archive, which every Block's `firmware/NOTICE` names. |

Sparse parts render as they are. A missing summary, tag, doc or seller shows as missing, never made up.

## How it builds

```sh
pnpm install --frozen-lockfile
scripts/site.sh --first-deploy
```

`scripts/site.sh` fetches the AVR runtime's source into the blob cache, builds `orbe`, writes this repo's registry with `orbe registry build . .scratch/site/registry`, builds orbe-wasm with `scripts/wasm.sh .scratch/site/wasm`, then runs `vite build` in `apps/site`. It needs what [wasm.md](wasm.md#build) needs, Node 24 and pnpm, and installs nothing. It requires either `--previous <dir>` or `--first-deploy`. `--previous <dir>` is the registry the live site serves, and the build keeps every version that registry served, as [registry.md](registry.md#versions) says. `--first-deploy` is the first deploy, which has no earlier registry.

The site is a [TanStack Start](https://tanstack.com/start/latest) app, set up as the website is: file routes in `src/routes`, `@cloudflare/vite-plugin`, Tailwind 4 and `@vitejs/plugin-react`.

- **Inputs.** `build/inputs.ts` is a Vite plugin. It runs `scripts/brand.sh check` and stops on a brand copy that differs from its list. It reads the registry, orbe-wasm and the runtime source from `.scratch/site`. It then writes `.scratch/site/public`, the folder Vite serves at `/embedded/` and copies into the build: the brand's public files, the registry at `r/`, the package at `orbe-wasm/`, Syne's licence beside the font, the runtime source at `sources/`, each part's `view.json` and `_headers`. The catalog reaches the server side as the module `virtual:orbe-site`, and only `src/site.server.ts` imports it.
- **Pages.** Vite's `base` is `/embedded/`, and Start makes that the router's basepath, so a route file such as `src/routes/parts/$id.tsx` answers `/embedded/parts/<id>/`. Each route's loader is a server function in `src/data.ts` that returns what the page shows. Start prerenders every page at build time: it starts from `/embedded/` and the 404 page, and follows each link it finds (`prerender.crawlLinks`). It renders in a Worker built from Start's server entry, and only the build runs that Worker. A loader's result is in the page's HTML, and the browser hydrates the page from it. Pages link to each other with plain anchors, so each navigation loads the next page's HTML and the browser never calls a server function. `scripts/check-site.sh` fails when a page and the registry disagree on how many parts and Blocks there are.
- **A part's 3D.** The build runs orbe-wasm's `partScene` in Node on each part's files and those of every part it is built from, and writes the scene message to `/embedded/parts/<id>/view.json`, with each model pointed at its file in the registry. The part page draws it with `packages/viewer`, as `orbe show` draws a scene, with no Worker and no WebAssembly. The parts list shows no images.
- **A Block.** The player runs `packages/viewer`'s `workerSource` on orbe-wasm's `worker.js` from `/embedded/orbe-wasm/`, and `loadBlock` reads the Block from `/embedded/r/` and checks every hash, as [viewer.md](viewer.md#run-a-block-in-the-page) says. It plays as soon as the Block opens. Showing another Block, or leaving the page, closes the source, which terminates the Worker and revokes its object URLs.
- **three.js** and the viewer are a chunk of their own, `src/components/live.tsx`, which only a page with a 3D view loads, once its text has drawn.
- **The brand.** [`packages/brand`](../packages/brand/README.md) is the website's brand, copied unchanged. Its `website/public/` files are served at `/embedded/`, and its `styles.css` holds the Tailwind 4 tokens.

### Install lines

Until the repo and the binaries are public, an install command would fail, so the site shows none. `ORBE_SITE_INSTALL=1 scripts/site.sh --first-deploy` shows them: `orbe add <id>` on each part and Block page, `cargo install`, the releases page and the repo on the landing page, the repo in the footer, and a Block's README. Without it, each of those places says the CLI is coming. The build defines the flag as a literal, `__ORBE_SITE_INSTALL__`, so a build without it holds none of that text, in its pages or in its scripts.

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

`scripts/check-site.sh` runs the type check and the brand check. It builds the site with `ORBE_SITE_INSTALL=1` and fails unless every part and Block in the registry has a page with its `orbe add` line, and the landing page shows `cargo install` and links the repo. It builds the site again without the flag. That build fails the check if a file of the site's own shows `orbe add`, `cargo install` or the repo, or if it does not serve the runtime source that sketches.sh pins and every Block's NOTICE names. Then `@playwright/test` (`apps/site/e2e/site.spec.ts`) drives that build in headless Chromium, once on `vite preview` and once on `wrangler dev --local`:

- the landing page plays nano-button-led, and its tab for nano-led-fade terminates the first Worker and revokes every object URL it made. The page asks no server for data, loads every file from under `/embedded/`, and draws the wordmark in Syne from `/embedded/fonts/`;
- the parts search lists every part, and for ten queries shows the parts `orbe search --json` gives, in its order;
- led-5mm's page draws its model, lists its five files with their licences, serves a download whose SHA-256 is the record's, and lists DigiKey with its checked date and the not-affiliated line;
- nano-board's page draws the board with its children. The Uno is titled as a compatible board, shows `MIT OR Apache-2.0` on its `part.jsonc`, `model.py` and GLB, and links the clone schematic it is drawn from;
- nano-button-led's page plays, and holding `@btn1` lights `@led1`, which goes dark on release. Its files show their own licences: the image and `LICENSE-runtime` LGPL-2.1-or-later, `LICENSE-libc` BSD-3-Clause, the scene and the sketch MIT OR Apache-2.0;
- an unknown path answers 404 with the site's page, and `/embeddedfoo` is not the site's;
- a registry file answers 304 to its own ETag, a missing one answers 404 with no immutable cache, a hashed build file keeps for a year, and a record's `@` path answers 307.

It prints each page's requests and bytes, uncompressed and gzipped, and the time to the first Block frame. It finds Chromium as `check-viewer-e2e.sh` does. `--renders <dir>` saves screenshots. `scripts/check.sh` does not run it.

## Hosting

`apps/site/wrangler.jsonc` is a Worker with no code. The build writes its deploy config, `dist/client/wrangler.json`, which serves `dist/client` as Workers static assets mounted at `/embedded` by `assets.base_path` ([Serving a subdirectory](https://developers.cloudflare.com/workers/static-assets/routing/advanced/serving-a-subdirectory/)). It runs on two routes, `orbefab.com/embedded` and `orbefab.com/embedded/*`. A pattern that ends in `*` matches every suffix, so `orbefab.com/embedded*` would also take `/embeddedfoo` ([Routes](https://developers.cloudflare.com/workers/configuration/routing/routes/)). The website's Worker holds `orbefab.com` as a Custom Domain, and a route takes precedence over a Custom Domain on the paths it matches, so this Worker answers `/embedded` and every path under it, and the website answers the rest.

- `_headers` caches `/embedded/assets/*` for a year, since their names are their hashes, and `/embedded/fonts/*` for 30 days. A rule in `_headers` applies to every response at a matching path, a 404 included ([Headers](https://developers.cloudflare.com/workers/static-assets/headers/)), and nothing in `_headers` or `not_found_handling` tells a hit from a miss. So `/embedded/r/files/*` keeps the default, `public, max-age=0, must-revalidate`, and a browser revalidates each file against its ETag, which answers 304 while it is unchanged. A registry file that is missing for a moment, during a deploy, is never cached. Every file gets `X-Content-Type-Options: nosniff`, and the registry gets `Access-Control-Allow-Origin: *`, so any page can read it.
- Workers static assets answer a path that holds `@` with a 307 to the same path with `%40`. A registry record such as `led-5mm@0.1.0.json` therefore costs one redirect, which browsers and `loadBlock` follow.
- A path under `/embedded` that holds no file gets `404.html`, the site's own page, with a 404.
- A build is about 430 files and 16 MB, 7 MB of it the runtime source. The registry stores each file once by its hash, and nothing beyond that is optimised.

## Deploy

The first deploy and every later one run from a clean checkout of `main`, with Cloudflare credentials for the `orbefab.com` zone.

```sh
git status --short                 # nothing
pnpm install --frozen-lockfile
scripts/check-site.sh              # builds dist/ as it deploys, and checks it
cd apps/site
pnpm exec wrangler deploy          # reads dist/client/wrangler.json, which the build wrote
```

`check-site.sh` leaves the deploy build, without install lines, in `apps/site/dist`. `wrangler deploy` in `apps/site` follows `.wrangler/deploy/config.json`, which the build wrote, to `dist/client/wrangler.json`, so it uploads `dist/client` as assets and no Worker code. Do not pass `--config wrangler.jsonc`, which skips the built config. Keep a copy of `dist/client/r` from each deploy, and pass it to the next one as `scripts/check-site.sh --previous <copy>`, so the registry keeps serving every version it served. The first deploy passes no `--previous`, and the build runs as `scripts/site.sh --first-deploy`.

After a deploy:

- `https://orbefab.com/embedded/` shows this site and plays a Block, and `https://orbefab.com/` and `https://orbefab.com/embeddedfoo` still go to the website.
- `https://orbefab.com/embedded/r/index.json` is the registry's index, and `https://orbefab.com/embedded/parts/led-5mm/` downloads its files.
- `https://orbefab.com/embedded/sources/avr-1.8.6.tar.bz2` downloads, with the SHA-256 every Block NOTICE names.
- `curl -sI https://orbefab.com/embedded/assets/<file>` shows the year-long `Cache-Control`, and `curl -sI https://orbefab.com/embedded/r/files/<sha256>` shows an `ETag`.
- `https://orbefab.com/embedded/no-such-page` answers 404 with this site's page, not the website's.
