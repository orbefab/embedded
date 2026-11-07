# orbefab/embedded

A hardware simulator for AI agents. You describe a circuit as a scene file, flash it with real firmware, and drive it from the command line with `orbe`. Today it runs Arduino Nano and Uno scenes headless, or in a 3D page in the browser. The repo is private for now, and everything goes public later.

## Folders

| Folder | What it holds |
| --- | --- |
| [`crates/`](crates/) | The `orbe` CLI in Rust. `orbe` is the binary. `orbe-scene` reads and checks scenes and part files and turns them into a netlist. `orbe-session` is a live session. |
| [`engine/`](engine/) | The simulation engine in Rust: the circuit solver and the AVR and ESP32-S3 chip models. It has its own [`AGENTS.md`](engine/AGENTS.md). |
| [`parts/`](parts/) | The parts catalog. Each folder is one real part: `part.jsonc`, its 3D model and `asset.json`. |
| [`examples/`](examples/) | Scenes that run, each with its firmware and tests. |
| [`packages/viewer`](packages/viewer/) | The 3D scene viewer, in React and three.js. |
| [`apps/viewer`](apps/viewer/) | The page `orbe show` serves. |
| [`tools/asset-step`](tools/asset-step/) | Imports each part's 3D model and checks it against the part's pins. |
| [`scripts/`](scripts/) | The checks. `check.sh` gates every change. Run `scripts/bless.sh` after adding or changing a part. |
| [`skills/orbe`](skills/orbe/SKILL.md) | The stub an agent installs. |
| [`docs/`](docs/) | Finalized docs: [`decisions.md`](docs/decisions.md), [`viewer.md`](docs/viewer.md), [`asset-step.md`](docs/asset-step.md). |

## Build and check

You need Rust (pinned in `rust-toolchain.toml`), Node 24 and pnpm.

```sh
pnpm install --frozen-lockfile
pnpm --filter @orbe/viewer-page build
cargo build -p orbe
scripts/check.sh
```

An `orbe` built without the viewer page still runs every headless command, and `orbe show` then says how to build the page.

## Skills for agents

`orbe skills get core` prints how to use `orbe`, for the version installed, and `orbe skills list` lists the other skills. [`skills/orbe/SKILL.md`](skills/orbe/SKILL.md) is a thin stub that tells an agent to run it. Install the stub for the agents you use with `npx skills add orbefab/embedded`.

## License

The code is under either of the MIT license ([LICENSE-MIT](LICENSE-MIT)) or the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), at your option. A contribution you submit for inclusion is under the same two licenses, with no other terms.
