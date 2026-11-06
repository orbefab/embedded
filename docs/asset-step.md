# The asset step

The asset step brings each part's 3D model from `orbefab/models` into the part's folder in `parts/`. Contributors run it when a model changes. Users never run it.

## Run it

```bash
npm ci --prefix tools/asset-step
node tools/asset-step/asset-step.mjs --models ../models --commit <models commit> [<id> ...]
```

`--models` is a checkout of `orbefab/models`. The step reads `GLB/parts/<id>.glb` and `GLB/parts/<id>.anchors.json` at the commit with `git show`, so it never changes the checkout. With no ids, it runs every part in `parts/` that has a GLB at that commit. With no `--commit`, each part reruns from the commit its `asset.json` records.

`scripts/check-assets.sh <models checkout>` reruns every part that way and fails if any file under `parts/` changes. Run it on a clean tree.

## What it writes

For each part that passes every check, it writes three files:

- `<id>.glb`, the merged model.
- `<id>.anchors.json`, copied unchanged. A part with no leads of its own, such as `nano-board`, has none.
- `asset.json`, with the source repo and commit, the path, SHA-256 and size of each input, the SHA-256 and size of each file written, the box in mm, the kept nodes, the draw calls and what each check measured.

A part that fails a check gets no files. The step prints the check and the reason, and exits 1. Files from an earlier run stay as they were.

## The checks

Lengths are in the part frame: mm, +Z up, with the seating plane at z = 0.

| Check | Passes when |
| --- | --- |
| frame | The scene has one root node, and it carries only glTF's -90 degree turn about X. |
| anchors | Each lead in `pins` has exactly one anchor, and each anchor's lead is in `pins`. Each anchor's `node` and every node in its `also` exist once. The tip of `node`, the centroid of its vertices furthest along `axis`, is within 0.01 mm of `at`. |
| seating | No node reaches more than 0.01 mm below z = 0, except a node an anchor names and the nodes under it. |
| names | Each node that `view` and `joints` name exists once. |
| merge | The merge keeps the nodes that `view` and `joints` name, and the `node` and `also` of each `leg` or `socket` anchor. Every other node loses its name and joins one mesh per material, and identical meshes are shared. Each kept node exists once afterwards. |
| composition | Only for a part with children. Each child's merged model, placed by `layout` with its full rotation, stays within 0.01 mm of z = 0 or above, except at the child's leads. |

The step does not compare a composed part with the whole object the models repo also exports, such as the Nano. The models repo does that in its own `checks/nano_ref.py`.

## In check.sh

`cargo test` checks every catalog part with a GLB, without Node. The hashes and sizes in `asset.json` must match the files, the anchors' leads must equal the leads in `pins`, and every node that `view` and `joints` name must be a kept node.

`parts.lock` hashes every file in a part's folder. After a run, rewrite the lock fixture with `ORBE_BLESS=1 cargo test -p orbe-scene --test read`, copy the changed part folders and the fixture's `parts.lock` into `examples/first-target/`, and run `./scripts/check.sh`.
