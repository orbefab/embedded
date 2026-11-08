# The site

`apps/site` is the homepage at [orbefab.com/embedded](https://orbefab.com/embedded): what orbe is, with a Block playing live, the parts catalog, the Blocks, and the `@orbefab` registry itself. Every page is static and is built from one registry build of this repo, so a part or Block added here reaches the site at the next build with no change to the site.

| Path | What it shows |
| --- | --- |
| `/embedded/` | What orbe is, every Block live in 3D with a tab each, the Blocks and the boards. |
| `/embedded/parts/` | Every part, and a search over the fields `orbe search` reads, in its order. `?q=` holds the query. |
| `/embedded/parts/<id>/` | The part in 3D, its pins, params, ratings with their sources, what is not simulated, what it is built from and what uses it, every file with its size and licence as a download, its docs, where to buy it with the date each link was checked, and its versions. A part that names a maker says Orbefab is not affiliated with or endorsed by that maker. |
| `/embedded/blocks/` | Every Block. |
| `/embedded/blocks/<id>/` | The Block live in 3D, its parts at their versions, its scene, firmware source and tests, its licence and every file. |
| `/embedded/r/` | The registry `orbe registry build` writes, so `@orbefab`'s template is `https://orbefab.com/embedded/r/{name}.json`. |

Sparse parts render as they are. A missing summary, tag, doc or seller shows as missing, never made up.

## How it builds

```sh
pnpm install --frozen-lockfile
scripts/site.sh
```

`scripts/site.sh` builds `orbe`, writes this repo's registry with `orbe registry build . .scratch/site/registry`, builds orbe-wasm with `scripts/wasm.sh .scratch/site/wasm`, then runs `apps/site/build.mjs` on both. It needs what [wasm.md](wasm.md#build) needs, Node 24 and pnpm, and installs nothing. Arguments go to `orbe registry build`, so `scripts/site.sh --previous <dir>` keeps every version an earlier build served, as [registry.md](registry.md#versions) says.

`build.mjs` first runs `scripts/brand.sh check` and refuses a brand copy that differs from its list. Then it builds the client with Vite and renders every page with React on the server side. It copies the registry to `dist/embedded/r/`, the package to `dist/embedded/orbe-wasm/` and Syne's licence beside the font, and writes `dist/_headers`. Last, it prints the file count and the bytes, by folder, and warns when orbe-wasm was not built from a clean commit.

- **Pages.** Each page is HTML with no script, except its islands: the parts search, a part's 3D model and the Block player. `src/islands/index.tsx` lists them. The client script hydrates each one from the props its HTML holds. three.js and the viewer are a chunk of their own, which only a page with a 3D view loads, once its text has drawn.
- **A part's 3D.** The build runs orbe-wasm's `partScene` in Node on each part's files and those of every part it is built from, and writes the scene message to `dist/embedded/parts/<id>/view.json`, with each model pointed at its file in the registry. The part page draws it with `packages/viewer`, as `orbe show` draws a scene, with no Worker and no WebAssembly. The parts list shows no images.
- **A Block.** The player runs `packages/viewer`'s `workerSource` on orbe-wasm's `worker.js` from `/embedded/orbe-wasm/`, and `loadBlock` reads the Block from `/embedded/r/` and checks every hash, as [viewer.md](viewer.md#run-a-block-in-the-page) says. It plays as soon as the Block opens. Showing another Block, or leaving the page, closes the source, which terminates the Worker and revokes its object URLs.
- **The brand.** [`packages/brand`](../packages/brand/README.md) is the website's brand, copied unchanged. Its `website/public/` is the site's public folder, and its `styles.css` holds the Tailwind 4 tokens.

### Install lines

Until the repo and the binaries are public, an install command would fail, so the site shows none. `ORBE_SITE_INSTALL=1 scripts/site.sh` shows them: `orbe add <id>` on each part and Block page, `cargo install` and the releases page on the landing page, a Block's README, and the links to the repo. Without it, each of those places says the CLI is coming.

## Look at it

```sh
node apps/site/preview.mjs        # http://127.0.0.1:4173/embedded/
```

`preview.mjs` serves `dist/` as Workers static assets do with this config: a folder's `index.html` at its path with a trailing slash, a redirect to add the slash, and the nearest `404.html` otherwise. It reads no `_headers`. `pnpm dlx wrangler@4.158.0 dev --local` in `apps/site` serves the same folder with Cloudflare's own asset handling and the headers, and talks to no Cloudflare service.

## Check it

`scripts/check-site.sh` runs the type check and the brand check, then builds the site with `ORBE_SITE_INSTALL=1` and fails unless every part and Block page shows its `orbe add` line and the landing page its `cargo install`. It builds the site again without it and fails if any page shows `orbe add`, `cargo install` or a link to the repo. Then `apps/site/e2e/site.mjs` drives the build in headless Chromium:

- the landing page plays nano-button-led, and its tab for nano-led-fade terminates the first Worker and revokes every object URL it made;
- the parts search lists every part, and for ten queries shows the parts `orbe search --json` gives, in its order;
- led-5mm's page draws its model, lists its five files with their licences, serves a download whose SHA-256 is the record's, and lists DigiKey with its checked date and the not-affiliated line;
- nano-board's page draws the board with its children, and the Uno is titled as a compatible board;
- nano-button-led's page plays, and holding `@btn1` lights `@led1`, which goes dark on release;
- an unknown path answers 404 with the site's page.

It prints each page's requests and bytes, uncompressed and gzipped, and the time to the first Block frame. It finds Chromium as `check-viewer-e2e.sh` does. `--renders <dir>` saves screenshots, and `--base <url>` drives a server of your own, such as `wrangler dev`. `scripts/check.sh` does not run it.

## Hosting

`apps/site/wrangler.jsonc` is a Worker with no code. It serves `dist/` as Workers static assets on the route `orbefab.com/embedded*`. The website's Worker holds `orbefab.com` as a Custom Domain, and a route runs before a Custom Domain on the paths it matches, so this Worker answers every path under `/embedded` and the website answers the rest.

- `dist/_headers` caches `assets/*` and `r/files/*` for a year, since their names are their hashes, and `fonts/*` for 30 days. Every other file revalidates on each use, since it keeps its URL across deploys. Every file gets `X-Content-Type-Options: nosniff`, and the registry gets `Access-Control-Allow-Origin: *`, so any page can read it.
- Workers static assets answer a path that holds `@` with a 307 to the same path with `%40`. A registry record such as `led-5mm@0.1.0.json` therefore costs one redirect, which browsers and `loadBlock` follow.
- A build is about 400 files and 8 MB. The registry stores each file once by its hash, and nothing beyond that is optimised.

## Deploy

The first deploy and every later one run from a clean checkout of `main`, with Cloudflare credentials for the `orbefab.com` zone.

```sh
git status --short                 # nothing
pnpm install --frozen-lockfile
scripts/check-site.sh              # builds dist/ as it deploys, and checks it
cd apps/site
pnpm dlx wrangler@4.158.0 deploy
```

`check-site.sh` leaves the deploy build, without install lines, in `apps/site/dist`. Keep a copy of `dist/embedded/r` from each deploy, and pass it to the next one as `scripts/check-site.sh --previous <copy>`, so the registry keeps serving every version it served. The first deploy has none.

After a deploy:

- `https://orbefab.com/embedded/` shows this site and plays a Block, and `https://orbefab.com/` still shows the website.
- `https://orbefab.com/embedded/r/index.json` is the registry's index, and `https://orbefab.com/embedded/parts/led-5mm/` downloads its files.
- `curl -sI https://orbefab.com/embedded/r/files/<sha256>` shows the year-long `Cache-Control`.
- `https://orbefab.com/embedded/no-such-page` answers 404 with this site's page, not the website's.
