# orbefab/embedded

Orbefab's **Embedded systems** branch: open-source mechatronics software built for AI agents. The repo is private for now; all tools go public eventually.

## What will live here

- **engine**: the simulator. Today it's `orbefab/sim-engine` ("micro-sim" was only a working name). Inside this repo it's just "engine". sim-engine isn't renamed; it gets archived once its code has moved.
- **maker-sandbox**: the learning UI (web). Once both are here it builds against the current engine instead of a frozen copy.
- **Orbe Blocks**: comes later, after the architecture and core decisions.
- Skills, examples and parts (sim model, specs, 3D, 2D) follow.

Right now the repo holds docs only. There's no code and no license file yet (the license is still open).

## How we proceed

1. **Investigations first.** See [`docs/investigations/`](docs/investigations/README.md). They run only when Alejandro says.
2. **Then migrate:** engine first, then sandbox, then the rest.

## Docs

- [`docs/decisions.md`](docs/decisions.md): what's decided so far.
- [`docs/investigations/README.md`](docs/investigations/README.md): investigations to run before migrating code.
- [`docs/open-questions.md`](docs/open-questions.md): what's still open.

The vision docs live in `orbefab/notes` (`org/VISION.md` and `org/branches/embedded-systems.md`). If anything here disagrees with them, the notes win.
