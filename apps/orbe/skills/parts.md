# orbe: parts

How a project keeps its parts: what `orbe add` copies, how `parts.lock` holds them, what each version error means, and the rules for a part's `buy` field. `orbe skills get core` covers finding a part and wiring it.

## Copies and the lock

`orbe add <id>…` copies parts from the catalog built into this `orbe` into `parts/<id>/` and records each in `parts.lock`, with its version and a hash of its files. It also copies the parts a part is built from, such as the chip on a board. Commit `parts/` and `parts.lock` with the scene, so the project checks the same on another machine.

```console
$ orbe add resistor-axial
wrote parts/resistor-axial 0.1.1
wrote parts.lock
```

Don't edit a copied part. A copy that differs, even in spacing, is a `part_modified` error. A board built from other parts, such as `nano-board`, is the exception. An edit to it checks, with a note. To change a part, copy it to a folder of your own and name it in the scene by its `./` path, as `"type": "./my-resistor"`.

## Versions

`orbe check` compares each copy with its `parts.lock` line and with the part this orbe carries:

- A copy that matches its lock line and is an older version than this orbe carries is a `part_outdated` error. The message names both versions. Its hint is one `orbe add` command that names every such id. Run it as printed, then check again.
- A copy that matches its lock line and is a newer version than this orbe carries is a `part_newer` error. The message names the version the project holds and the version this orbe carries. The hint says this project needs a newer orbe, and that id is not in an `orbe add` command.
- `--overwrite` leaves a newer copy where it is, including when the command writes a board that is built from it, and says this project needs a newer orbe for it. When `orbe add` refuses, it says the same for each copy it left, and the `orbe add` in its hint names every id that command would write. That command always names an id. When nothing can be written, the hint names no `orbe add` and says this project needs a newer orbe.
- The same version with different bytes is still `part_modified` for a sealed part: a registry refuses that change and asks for a new version. A board stays the exception above.

## Blocks

`orbe add <block>` installs a Block, a small circuit that works. It copies the parts the scene names into `parts/` and records them in `parts.lock`, then writes the Block's `scene.jsonc`, `firmware/`, `tests/` and `README.md`. A folder that already holds another `scene.jsonc` is `scene_exists`, since orbe does not merge scenes. A file already there with other bytes is `file_exists`. Either refusal writes nothing. A file already there with the Block's bytes is kept, so running the same install again finishes one that stopped part way. A new folder is only for a file whose bytes differ.

## Finding a part

`orbe search <words>` lists parts whose id, title, maker, number, summary or tags hold every word, then Blocks whose id, title, description or categories do. An exact id comes first. Then a part whose every word is a whole word of its id, its title or its tags. Then the rest. Ids break ties. `orbe search <words> --json` puts the parts under `data.parts` and the Blocks under `data.blocks`. `orbe search` with no words lists every part a scene can place, by its first category, then the Blocks.

`orbe part <id>` prints one catalog part: its title, maker and number, license, folder hash, pins, params, ratings, what is not simulated, datasheets, where to buy it, files, the parts it is built from and the parts that use it. It also prints one line on how the part plugs into a breadboard. A rating set to null prints as unrated. A part with no `buy` prints `none`. A name that is not an id, and is a whole word of exactly one id, prints that part or that Block, so `orbe part 1n4007` prints `diode-1n4007`. When several ids share that word, the command is `unknown_part` and the hint lists those ids. `orbe part <block>` prints a Block and the command that installs it. `orbe part <id> --json` prints the card as JSON.

## The `buy` field

`buy` is optional on a part. Leave it out when there is no link. An empty list is an error. Each entry has `seller`, `url` and `checked`. `checked` is the day the link was checked, as `YYYY-MM-DD`. `seller` is a name in `packages/scene/buy-sellers.jsonc`, and the URL's host is one of that seller's hosts. A new seller is a one-line addition to that file, reviewed for affiliate and tracking links. `url` is a lowercase `https://` URL with no userinfo, no whitespace, no query and no fragment. Userinfo is an `@` before the host. An entry has no price. A repeated entry is an error.

## Errors

| Code | What to do |
| --- | --- |
| `part_missing` | Run the `orbe add` command in the hint. |
| `unknown_part` | No Block or part has that id. Pick one the hint names, or find one with `orbe search <words>`. |
| `busy` | Another `orbe add` is at work in this project. Wait for it, then run yours again. |
| `scene_exists`, `file_exists` | `orbe add <block>` found another scene, or a file whose bytes differ. Matching files are kept, and running it again finishes an install that stopped part way. Use a new folder only when the bytes differ, as the hint shows. |
| `symlink` | `orbe add <block>` found a link where it would write. It does not follow the link. Remove the link, then run the command in the hint again. |
| `part_outdated` | The copy matches its `parts.lock` line and is an older version than this orbe carries. Run the one `orbe add` command in the hint. It names every older id, and no newer one. |
| `part_newer` | The copy matches its `parts.lock` line and is a newer version than this orbe carries. This project needs a newer orbe. The hint names no `orbe add` command. |
| `part_modified` | A copied part was edited. Run `orbe add <id> --overwrite` to restore it, or copy it to a folder of your own. |
| `part_format` | The part file's format is not one this orbe reads. An older catalog copy is written for an older orbe: run `orbe add <id> --overwrite`. A part opened by a `./` path is the user's file: update that file to the format this orbe reads, including its model, or point the scene at the catalog id if the file copies one. A newer part means this project needs a newer orbe. This orbe reads part formats 1 and 2. Format 2 is format 1 plus `"as": "blob"` on a file param, and a blob param in a format 1 file is `scene.param`, with the hint to set `"format": 2`. A scene is format 1. |
