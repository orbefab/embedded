# AGENTS.md

- `docs/` holds only finalized docs. Raw plans, drafts and notes do not go into the repo.
- A core crate (`packages/cores`) never names a vendor's registers, and a chip crate (`packages/socs`) never decodes a CPU's instructions. [docs/engine/README.md](docs/engine/README.md#cores-and-chips) says why.
- Run `scripts/check.sh fast` before every commit, `web` when a change touches the viewer or the site, and `full` when it touches `packages/engine`, `packages/cores` or `packages/socs`. The headers of [scripts/check.sh](scripts/check.sh) and [.github/workflows/ci.yml](.github/workflows/ci.yml) say what each tier runs and what CI runs when.
