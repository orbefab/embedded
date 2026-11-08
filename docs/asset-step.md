# The asset step

The asset step checks a part's body model and writes the merged GLB and `asset.json`. A part with `model.py` is built in this repo. Any other part is read from the commit its `asset.json` records in `orbefab/models`. Contributors run this when a model changes. Users never run it.

## A part built here

```bash
scripts/model.sh led-5mm
scripts/model.sh --all
```

Rebuild only the parts you changed. A second run that writes the same bytes is proven on Linux x86_64.

`scripts/model.sh` runs `parts/<id>/model.py`. The script uses build123d and the helpers in `packages/cad`. It writes the raw GLB and the anchors into a temporary folder, then runs the asset step with `--local --raw` on that folder. `--all` builds every part that has `model.py`.

The shell script checks that uv is installed, that Node is 24 or newer, that the asset step's dependencies are installed, and that `packages/cad/.venv` can import build123d. When one is missing it prints the install command and exits. It never installs anything itself. The model environment is Python 3.13. Install it with `uv sync --project packages/cad --frozen`.

`--local` reads the raw GLB and anchors from `--raw`, runs the checks, and writes the merged GLB, `<id>.source.glb`, the anchors and `asset.json` into the part folder only after that part's checks pass. `<id>.source.glb` is that raw export. `source.sha256` is the SHA-256 of `model.py`. The hashes under `files` cover the merged GLB, the anchors and `<id>.source.glb`. With no ids, `--local` runs every part that has `model.py`, and it still needs `--raw`. It does not read the GLB already in the part folder.

After it finishes, run `scripts/bless.sh`.

The model build stays off CI.

## A part still recorded by commit

```bash
pnpm install --frozen-lockfile
node packages/asset-step/asset-step.mjs --models ../models [<id> ...]
```

`--models` is a checkout of `orbefab/models`. The step reads `GLB/parts/<id>.glb` and `GLB/parts/<id>.anchors.json` at a commit with `git show`, so it never changes the checkout. With no `--commit`, each part reruns from the commit its `asset.json` records, and fails if the checkout lacks that commit, which you then fetch. With no ids, it runs every part in `parts/` that has a GLB at that commit, and skips a part that has `model.py`. Naming such a part fails and tells you to run `scripts/model.sh`.

With no `--commit` and no ids, a part with no GLB at its commit, or with no `asset.json` to name a commit, is skipped. A part with no model yet does not fail the run.

`scripts/check-assets.sh <models checkout>` runs the step's tests, rebuilds every part that has `model.py`, then reruns every other part from its recorded commit, and fails if any file under `parts/` changes. The script still needs the models checkout for the other parts, and the `packages/cad` environment for the rebuild. Run it on a clean tree. `check.sh` does not run it.

## What it writes

For each part that passes every check, it writes:

- `<id>.glb`, the merged model.
- `<id>.source.glb`, only for a part built here. It is the export before the merge. Composition of a parent reads it so a pad keeps its name.
- `<id>.anchors.json`, copied unchanged. A part with no leads of its own, such as `nano-board`, has none.
- `asset.json`, with the source, the SHA-256 and size of each file written, the box in mm, the kept nodes, the draw calls and what each check measured. A part built here records `source.script` as `model.py` and `source.sha256` as that file's SHA-256. A part still read from `orbefab/models` records that repo, the commit, and each input's path, SHA-256 and size.

A part that fails a check gets no files. The step prints the check and the reason, goes on to the next part, and exits 1 at the end. Files from an earlier run stay as they were. An input the step cannot read, such as an anchors file with no `also` list, fails its part the same way.

## The checks

Lengths are in the part frame: mm, +Z up, with the seating plane at z = 0.

A lead is the mesh of the node an anchor names. The anchors check takes a lead's tip from that node's own vertices, so a node whose mesh sits on a child node fails. Seating and composition exempt the nodes the anchors name in `node` and `also`, and not the nodes under them. A vertex whose coordinates are not finite numbers fails on any node, the exempt nodes included. On a lead's own node the anchors check, which runs first, fails it; elsewhere seating and composition do.

| Check | Passes when |
| --- | --- |
| frame | The scene has one root node, and it carries only glTF's -90 degree turn about X. |
| anchors | Each anchor has an integer `lead`, a `node` name, an `also` list of names, an `at` and a unit `axis` of three numbers each, and a `kind` of leg, socket or pad. Each lead in `pins` has exactly one anchor, and each anchor's lead is in `pins`. Each anchor's `node` and every node in its `also` exist once. `node` has vertices of its own. Its tip, the centroid of its vertices within 0.001 mm of the furthest along `axis`, is within 0.01 mm of `at`. |
| seating | No node reaches more than 0.01 mm below z = 0, except the nodes an anchor names. |
| names | Each node that `view` and `joints` name exists once. |
| merge | The merge keeps the nodes that `view` and `joints` name, and the `node` and `also` of each `leg` or `socket` anchor. Every other node loses its name and joins one mesh per material, and identical meshes are shared. Each kept node exists once afterwards, and the scene still has nodes to draw. |
| composition | Only for a part with children. The part's model, each child's model and each model below those, placed by each parent's `layout` with its full rotation, stay within 0.01 mm of z = 0 or above, except at the nodes their anchors name, pads included. An `at` of two numbers sits at z = 0. A child with `model.py` is `<id>.source.glb` in its folder when that file exists, and the merged GLB otherwise. Any other child is its source GLB at the commit its `asset.json` records, which passed these checks. The merge drops a pad's name, so a pad is a lead in that source GLB and not in the merged file. A child whose recorded commit the models checkout lacks fails the part, and so does a part that holds itself. |

The step does not compare a composed part with the whole object the models repo also exports, such as the Nano.

Composition needs each child's `asset.json`. A part with children and no body of its own has none, so a part that holds one fails composition, and the children below it are not placed. `part.jsonc` does not say whether a part has a body, so the step cannot tell such a part from one whose model has not been brought in yet.

## In check.sh

`cargo test` checks every catalog part with a GLB, without Node. The hashes and sizes in `asset.json` must match the files. When the part has `model.py`, that file's SHA-256 must equal `source.sha256`, and a mismatch tells you to run `scripts/model.sh <id>`. The anchors' leads must equal the leads in `pins`, and each anchor's `at` must sit inside the box `asset.json` records, with a unit `axis`. The test reads the node names from the GLB's JSON chunk. They must equal `kept_nodes`, and `kept_nodes` must equal what the merge keeps by `part.jsonc` and the anchors, with each leg or socket anchor's `node` and `also` in the GLB once. The test reads no vertices, so the geometry checks stay with the asset step and `scripts/check-assets.sh`.

`parts.lock` hashes every file in a part's folder. After `scripts/model.sh`, run `scripts/bless.sh`. It rewrites the lock fixture from `parts/` and refreshes each example's part copies and `parts.lock` from the catalog ids its scene names. A copy the scene no longer names is removed. Then run `./scripts/check.sh`.
