# Contributing

You need Rust (pinned in `rust-toolchain.toml`), Node 24 and pnpm, and what the check you run needs. The header of [`scripts/check.sh`](../scripts/check.sh) lists that for each tier.

```sh
pnpm install --frozen-lockfile
pnpm --filter @orbe/viewer-page build
cargo install --locked --path apps/orbe
scripts/check.sh fast
```

`cargo install` puts `orbe` on `PATH`. An `orbe` built without the viewer page still runs every headless command, and `orbe show` then says how to build the page. `ORBE_REGISTRY` replaces the registry URL template, which is how a test points `orbe add` at a registry built from this tree. `orbe add`, `orbe search` and `orbe part` need curl. The header of `scripts/check.sh` says what each tier runs, and the header of [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) says what CI runs, and when.

## Folders

| Folder | What it holds |
| --- | --- |
| [`apps/orbe`](../apps/orbe/) | The `orbe` CLI in Rust. `orbe` is the binary. |
| [`apps/viewer`](../apps/viewer/) | The page `orbe show` serves. |
| [`apps/site`](../apps/site/) | The homepage at orbefab.com/orbe: the parts catalog, the Blocks live in 3D, and the registry. [`docs/site.md`](site.md) says how to build, check and deploy it. |
| [`packages/scene`](../packages/scene/) | `orbe-scene` reads and checks scenes and part files and turns them into a netlist. |
| [`packages/session`](../packages/session/) | `orbe-session` is a live session. |
| [`packages/wasm`](../packages/wasm/) | `orbe-wasm` runs a Block live in a browser Web Worker. [`docs/wasm.md`](wasm.md) says how to build and load it. |
| [`packages/registry`](../packages/registry/) | `orbe-registry` builds a registry of parts and Blocks for `orbe registry build`. [`docs/registry.md`](registry.md) is the contract a client reads it by. |
| [`packages/engine`](../packages/engine/) | The simulation engine in Rust: the netlist and run folder, the circuit solver, and its test circuits and firmware. [`docs/engine`](engine/README.md) says how it fits together. |
| [`packages/cores`](../packages/cores/) | Instruction-set cores. `xtensa` runs the ESP32 and ESP32-S3 cores, and `avr` runs the AVR instruction set. A core never names a vendor's registers. |
| [`packages/socs`](../packages/socs/) | Chips around a core. `espressif` is the ESP32-S3 and `microchip` is the ATmega328P. A chip never decodes a CPU's instructions. |
| [`packages/scratch`](../packages/scratch/) | `orbe-scratch`, the temp folder the tests use. It is removed when it drops, even when a test panics, and folders a test locked are opened first. |
| [`packages/viewer`](../packages/viewer/) | The 3D scene viewer, in React and three.js. |
| [`packages/brand`](../packages/brand/README.md) | The Orbefab brand, copied unchanged from the website repo, with `scripts/brand.sh` to check and refresh the copy. |
| [`packages/asset-step`](../packages/asset-step/) | Imports each part's 3D model and checks it against the part's pins. |
| [`packages/cad`](../packages/cad/) | Helpers for part models. A part's `model.py` uses build123d and these helpers. |
| [`parts/`](../parts/) | The parts catalog. Each folder is one real part: `part.jsonc`, `model.py` when the model is built here, its 3D model and `asset.json`. |
| [`blocks/`](../blocks/) | Blocks: small circuits that work, each with its scene, firmware and tests. `orbe add <block>` installs one into a project. |
| [`registry.jsonc`](../registry.jsonc) | Names the official registry, `@orbefab`, that `orbe registry build .` builds from `parts/` and `blocks/`. |
| [`scripts/`](../scripts/) | The checks. `check.sh` runs them in tiers, and its header says what each tier runs. `scripts/check-assets.sh` and `scripts/check-release.sh`, which runs on a release archive, are in no tier. `scripts/release-build.sh` builds a release binary with the viewer, and `scripts/install.sh` is the install script the site serves. `scripts/check-release.sh` checks that archive's sha256, installs it with `scripts/install.sh --tarball` and refuses a wrong checksum, and runs a newcomer's first commands on the installed `orbe`, including the viewer page, its session socket and one part model. `scripts/model.sh` builds a part's model. Run `scripts/bless.sh` after changing a viewer message type, to rewrite the viewer's `protocol.ts`. `scripts/registry.sh` builds the registry the site publishes, from the tags `v0.2.0` and later. |
| [`skills/orbe`](../skills/orbe/SKILL.md) | The stub an agent installs. |
| [`docs/`](./) | Finalized docs: [`decisions.md`](decisions.md), [`viewer.md`](viewer.md), [`asset-step.md`](asset-step.md), [`registry.md`](registry.md), [`wasm.md`](wasm.md), [`site.md`](site.md), [`engine/`](engine/README.md). |

Pull requests are welcome. Before you open one, run the checks [`AGENTS.md`](../AGENTS.md) names for what you changed.
