# orbefab/embedded

Orbefab's **Embedded systems** branch: open-source mechatronics software built for AI agents. The repo is private for now; all tools go public eventually.

## What will live here

- **engine**: the simulator. Today it's `orbefab/sim-engine` ("micro-sim" was only a working name). Inside this repo it's just "engine". sim-engine isn't renamed; it gets archived once its code has moved.
- **maker-sandbox**: the learning UI (web). Once both are here it builds against the current engine instead of a frozen copy.
- **Orbe Blocks**: comes later, after the architecture and core decisions.
- Skills, examples and parts (sim model, specs, 3D) follow.

The engine lives in [`engine/`](engine/), with its own [`AGENTS.md`](engine/AGENTS.md) and docs. The rest is docs for now.

## License

The code is under either of the MIT license ([LICENSE-MIT](LICENSE-MIT)) or the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), at your option. A contribution you submit for inclusion is under the same two licenses, with no other terms.

## How we proceed

1. **Investigations first.** See [`docs/investigations/`](docs/investigations/README.md).
2. **Then migrate:** engine first, then sandbox, then the rest.

## Docs

- [`docs/decisions.md`](docs/decisions.md): what's decided so far.
- [`docs/investigations/README.md`](docs/investigations/README.md): investigations to run before migrating code.
- [`docs/open-questions.md`](docs/open-questions.md): what's still open.
