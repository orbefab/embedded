# orbe: parts

How a project keeps its parts: what `orbe add` copies, the copy the project owns, versions, Blocks and the `buy` field. `orbe skills get core` covers finding a part and wiring it.

## The copy

`orbe add <id>…` copies parts from the catalog built into this `orbe` into `parts/<id>/`. It also copies the parts a part is built from, such as the chip on a board. One id has one folder. `id@version` copies that catalog version when the catalog has it. A version the catalog does not have is `unknown_part`. Commit `parts/` with the scene.

```console
$ orbe add resistor-axial
wrote parts/resistor-axial 0.1.1
```

The copy is the project's. Edit it. `orbe check` reads `parts/<id>/` as it is and does not compare it to the catalog. To use a part of your own, put it in a folder and name that folder in the scene by its `./` path, as `"type": "./my-resistor"`.

A folder already at `parts/<id>/` is `part_exists`. The command writes nothing. The message names the copy's version when `part.jsonc` has one, as `parts/led-5mm is already led-5mm 0.1.1`. Remove the folder and run the `orbe add` command in the hint.

## Blocks

`orbe add <block>` installs a Block, a small circuit that works. It copies the parts the scene names into `parts/`, then writes the Block's `scene.jsonc`, `firmware/`, `tests/` and `README.md`. A part folder already there is `part_exists`, and the command writes nothing. A folder that already holds another `scene.jsonc` is `scene_exists`, since orbe does not merge scenes. A file already there with other bytes is `file_exists`. Either refusal writes nothing. A file already there with the Block's bytes is kept when the part copies succeed, so running the same install again finishes one that stopped before those copies.

## Finding a part

`orbe search <words>` lists parts whose id, title, maker, number, summary or tags hold every word, then Blocks whose id, title, description or categories do. An exact id comes first. Then a part whose every word is a whole word of its id, its title or its tags. Then the rest. Ids break ties. `orbe search <words> --json` puts the parts under `data.parts` and the Blocks under `data.blocks`. `orbe search` with no words lists every part a scene can place, by its first category, then the Blocks.

`orbe part <id>` prints one catalog part: its title, maker and number, license, folder hash, pins, params, ratings, what is not simulated, datasheets, where to buy it, files, the parts it is built from and the parts that use it. It also prints one line on how the part plugs into a breadboard. A rating set to null prints as unrated. A part with no `buy` prints `none`. A name that is not an id, and is a whole word of exactly one id, prints that part or that Block, so `orbe part 1n4007` prints `diode-1n4007`. When several ids share that word, the command is `unknown_part` and the hint lists those ids. `orbe part <block>` prints a Block and the command that installs it. `orbe part <id> --json` prints the card as JSON.

## The `buy` field

`buy` is optional on a part. Leave it out when there is no link. An empty list is an error. Each entry has `seller`, `url` and `checked`. `checked` is the day the link was checked, as `YYYY-MM-DD`. `seller` is a name in `packages/scene/buy-sellers.jsonc`, and the URL's host is one of that seller's hosts. A new seller is a one-line addition to that file, reviewed for affiliate and tracking links. `url` is a lowercase `https://` URL with no userinfo, no whitespace, no query and no fragment. Userinfo is an `@` before the host. An entry has no price. A repeated entry is an error.

## Errors

| Code | What to do |
| --- | --- |
| `part_missing` | The folder `parts/<id>/` is not there. Run the `orbe add` command in the hint. |
| `part_exists` | `parts/<id>/` is already there, so `orbe add` wrote nothing. Remove it and run the command in the hint. |
| `unknown_part` | No Block or part has that id. Pick one the hint names, or find one with `orbe search <words>`. |
| `busy` | Another `orbe add` is at work in this project. Wait for it, then run yours again. |
| `scene_exists`, `file_exists` | `orbe add <block>` found another scene, or a file whose bytes differ. Matching files are kept when the part copies succeed, and running it again finishes an install that stopped before those copies. Use a new folder only when the bytes differ, as the hint shows. |
| `symlink` | `orbe add <block>` found a link where it would write. It does not follow the link. Remove the link, then run the command in the hint again. |
| `part_format` | The part file's format is not one this orbe reads. An older catalog copy is this project's file: remove the folder and run `orbe add <id>`. A part opened by a `./` path is the user's file: update that file to the format this orbe reads, including its model, or point the scene at the catalog id if the file copies one. A newer part means this project needs a newer orbe. This orbe reads part formats 1, 2 and 3. Format 2 is format 1 plus `"as": "blob"` on a file param, and a blob param in a format 1 file is `scene.param`, with the hint to set `"format": 2`. Format 3 is format 2 plus an addressable LED's `DOUT` and its colour reading. A scene is format 1. |
