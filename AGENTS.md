# AGENTS.md

- `docs/` holds only finalized docs. Raw plans, drafts and notes do not go into the repo.
- A core crate (`packages/cores`) never names a vendor's registers, and a chip crate (`packages/socs`) never decodes a CPU's instructions. [docs/engine/README.md](docs/engine/README.md#cores-and-chips) says why.
- Run `scripts/check.sh fast` before every commit. CI runs the engine, viewer and site checks on every pull request. Run `scripts/check.sh full`, which adds the browser speed check, before a release and on a change meant to make the engine faster. The headers of [scripts/check.sh](scripts/check.sh) and [.github/workflows/ci.yml](.github/workflows/ci.yml) say what each tier runs and what CI runs when.
