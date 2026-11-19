# orbefab/embedded

A hardware simulator for AI agents. You describe a circuit as a scene file, flash it with real firmware, and drive it from the command line with `orbe`. Today it runs Arduino Nano and Uno scenes headless, or in a 3D page in the browser. The repo is private for now, and everything goes public later.

0.1 runs the Arduino Nano and Uno. The ESP32 cores are in progress.

## Folders

| Folder | What it holds |
| --- | --- |
| [`apps/orbe`](apps/orbe/) | The `orbe` CLI in Rust. `orbe` is the binary. |
| [`apps/viewer`](apps/viewer/) | The page `orbe show` serves. |
| [`apps/site`](apps/site/) | The homepage at orbefab.com/embedded: the parts catalog, the Blocks live in 3D, and the registry. [`docs/site.md`](docs/site.md) says how to build, check and deploy it. |
| [`packages/scene`](packages/scene/) | `orbe-scene` reads and checks scenes and part files and turns them into a netlist. |
| [`packages/session`](packages/session/) | `orbe-session` is a live session. |
| [`packages/wasm`](packages/wasm/) | `orbe-wasm` runs a Block live in a browser Web Worker. [`docs/wasm.md`](docs/wasm.md) says how to build and load it. |
| [`packages/registry`](packages/registry/) | `orbe-registry` builds a registry of parts and Blocks for `orbe registry build`. [`docs/registry.md`](docs/registry.md) is the contract a client reads it by. |
| [`packages/engine`](packages/engine/) | The simulation engine in Rust: the netlist and run folder, the circuit solver, and its test circuits and firmware. [`docs/engine`](docs/engine/README.md) says how it fits together. |
| [`packages/cores`](packages/cores/) | Instruction-set cores. `xtensa` runs the ESP32 and ESP32-S3 cores, and `avr` runs the AVR instruction set. A core never names a vendor's registers. |
| [`packages/socs`](packages/socs/) | Chips around a core. `espressif` is the ESP32-S3 and `microchip` is the ATmega328P. A chip never decodes a CPU's instructions. |
| [`packages/viewer`](packages/viewer/) | The 3D scene viewer, in React and three.js. |
| [`packages/brand`](packages/brand/README.md) | The Orbefab brand, copied unchanged from the website repo, with `scripts/brand.sh` to check and refresh the copy. |
| [`packages/asset-step`](packages/asset-step/) | Imports each part's 3D model and checks it against the part's pins. |
| [`packages/cad`](packages/cad/) | Helpers for part models. A part's `model.py` uses build123d and these helpers. |
| [`parts/`](parts/) | The parts catalog. Each folder is one real part: `part.jsonc`, `model.py` when the model is built here, its 3D model and `asset.json`. |
| [`blocks/`](blocks/) | Blocks: small circuits that work, each with its scene, firmware and tests. `orbe add <block>` installs one into a project. |
| [`registry.jsonc`](registry.jsonc) | Names the official registry, `@orbefab`, that `orbe registry build .` builds from `parts/` and `blocks/`. |
| [`scripts/`](scripts/) | The checks. `check.sh lint` is what CI runs, `check.sh fast` adds the tests and gates every change, `check.sh web` runs the viewer and the site, and `check.sh full` runs `fast`, the engine checks and `web`. `scripts/check-assets.sh`, which needs a models checkout, and `scripts/check-release.sh`, which runs on a release archive, stay outside `full`. `scripts/release-build.sh` builds a release binary with the viewer, and `scripts/install.sh` is the install script the site serves. `scripts/check-release.sh` checks that archive's sha256, installs it with `scripts/install.sh --tarball` and refuses a wrong checksum, and runs a newcomer's first commands on the installed `orbe`, including the viewer page, its session socket and one part model. `scripts/model.sh` builds a part's model. Run `scripts/bless.sh` after changing a viewer message type, to rewrite the viewer's `protocol.ts`. `scripts/released.sh` rewrites `released.lock` when a release is tagged. |
| [`skills/orbe`](skills/orbe/SKILL.md) | The stub an agent installs. |
| [`docs/`](docs/) | Finalized docs: [`decisions.md`](docs/decisions.md), [`viewer.md`](docs/viewer.md), [`asset-step.md`](docs/asset-step.md), [`registry.md`](docs/registry.md), [`wasm.md`](docs/wasm.md), [`site.md`](docs/site.md), [`engine/`](docs/engine/README.md). |

## Use orbe

```sh
curl -fsSL https://orbefab.com/embedded/install.sh | sh
```

The script installs `orbe`, with the 3D viewer, for Linux x86_64 and macOS arm64. It puts the binary in `~/.local/bin`. When that directory is not on `PATH`, the script says so. It does not edit your shell rc files.

### Try it

```sh
mkdir demo
cd demo
orbe add nano-button-led
orbe test
orbe open
```

`orbe add` copies the Block into the folder. `orbe test` runs its tests. `orbe open` opens the scene, paused. `orbe show` opens the 3D viewer on that session. `orbe part <id>` prints one catalog part, and `orbe search <words>` finds parts by id, title, maker, number, summary and tags.

## Contribute

You need Rust (pinned in `rust-toolchain.toml`), cargo-nextest at the version `.config/nextest.toml` names, uv, shellcheck, Node 24 and pnpm. `scripts/check.sh web` also needs the Chromium, Firefox and WebKit that Playwright 1.64.1 expects, installed with `pnpm exec playwright install chromium firefox webkit`. On Ubuntu, Firefox and WebKit also need their system libraries: `sudo pnpm exec playwright install-deps firefox webkit`.

```sh
pnpm install --frozen-lockfile
pnpm --filter @orbe/viewer-page build
cargo install --locked --path apps/orbe
scripts/check.sh fast
```

`scripts/check.sh web` runs the viewer unit tests, the viewer and site checks in Chromium, and a smoke of the viewer and the site in Firefox and WebKit. `scripts/check.sh full` runs `fast`, the engine checks, and `web`. `cargo install` puts `orbe` on `PATH`. An `orbe` built without the viewer page still runs every headless command, and `orbe show` then says how to build the page.

## Skills for agents

`orbe skills get core` prints how to use `orbe`, for the version installed, and `orbe skills list` lists the other skills. [`skills/orbe/SKILL.md`](skills/orbe/SKILL.md) is a thin stub that tells an agent to run it. Install the stub for the agents you use with `npx skills add orbefab/embedded`.

## License

The code is under MIT OR Apache-2.0, at your option, unless a file says otherwise. The MIT license is [LICENSE-MIT](LICENSE-MIT). The Apache License, Version 2.0 is [LICENSE-APACHE](LICENSE-APACHE). A contribution you submit for inclusion is under the same two licenses, with no other terms.

The Block firmware images in `blocks/*/firmware/` and the engine's test sketch images in `packages/engine/tests/firmware/sketch/` say otherwise. Each image is statically linked to the Arduino AVR core, which is LGPL-2.1-or-later. See `firmware/NOTICE` beside those images.
