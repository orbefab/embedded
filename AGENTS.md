# AGENTS.md

- `docs/` holds only finalized docs. Raw plans, drafts and notes do not go into the repo.
- A core crate (`packages/cores`) never names a vendor's registers, and a chip crate (`packages/socs`) never decodes a CPU's instructions. [docs/engine/README.md](docs/engine/README.md#cores-and-chips) says why.
- Run `scripts/check.sh fast` before every commit, `scripts/check.sh web` when a change touches the viewer or the site, and `scripts/check.sh full` before a commit that touches `packages/engine`, `packages/cores` or `packages/socs`. `full` runs `web`. CI runs lint, the Rust tests, the viewer checks and the site suite on every pull request, and the engine extras when engine code changes. On main, when the repository variable `SITE_DEPLOY` is `1`, the site workflow also runs `scripts/check-site.sh` and deploys, as [docs/site.md](docs/site.md#deploy) describes.
