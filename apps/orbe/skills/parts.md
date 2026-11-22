# orbe: parts

How a project keeps its parts: what `orbe add` downloads, the copy the project owns, versions, Blocks and the `buy` field. `orbe skills get core` covers finding a part and wiring it.

## The copy

`orbe add <id>…` downloads the latest part from `https://orbefab.com/embedded/r/` into `parts/<id>/`. It also puts the parts a part is built from in `parts/<id>/parts/<dep>/`, such as the chip on a board. One id has one folder at the top of `parts/`. `id@version` is a usage error, and the hint is the bare id: `orbe add` installs the highest version. Commit `parts/` with the scene.

```console
$ orbe add resistor-axial
wrote parts/resistor-axial 0.1.1
```

The copy is the project's. Edit it. `orbe check`, `orbe open` and `orbe test` read `parts/<id>/` as it is and never use the network. To use a part of your own, put it in a folder and name that folder in the scene by its `./` path, as `"type": "./my-resistor"`.

A folder already at `parts/<id>/` for an id you named is `part_exists`. The command writes nothing. The message names the copy's version when `part.jsonc` has one, as `parts/led-5mm is already led-5mm 0.1.1`. Remove the folder and run the `orbe add` command in the hint. A part nested inside a board does not block adding that part on its own, and adding the board again after you remove `parts/<board>/` works while that part stays at the top of `parts/`.

`@<registry>/<id>` is a usage error. This orbe reads the `@orbefab` registry only.

## Blocks

`orbe add <block>` installs a Block, a small circuit that works. It downloads the parts the scene names into `parts/`, nesting each part's own parts inside that folder, then writes the Block's `scene.jsonc`, `firmware/`, `tests/` and `README.md`. It does not write `block.jsonc`. A part folder the scene names that is already there is `part_exists`, and the command writes nothing. The message names those folders, and the hint is the same `orbe add <block>`. Remove the folders the message names, then run that command. A folder that already holds another `scene.jsonc` is `scene_exists`, since orbe does not merge scenes. A file already there with other bytes is `file_exists`. Either refusal writes nothing. A file already there with the Block's bytes is kept when those part folders are not there yet.

## Finding a part

`orbe add`, `orbe search` and `orbe part` need curl. When the registry cannot be reached, each prints `registry` and names the URL.

`orbe search <words>` lists parts whose id, title, maker, number, summary or tags hold every word, then Blocks whose id, title, description or categories do. An exact id comes first. Then a part whose every word is a whole word of its id, its title or its tags. Then the rest. Ids break ties. `orbe search <words> --json` puts the parts under `data.parts` and the Blocks under `data.blocks`. `orbe search` with no words lists every part a scene can place, by its first category, then the Blocks.

`orbe part <id>` prints one part: its title, maker and number, license, folder hash, pins, params, ratings, what is not simulated, datasheets, where to buy it, files, and the parts it is built from. It also prints one line on how the part plugs into a breadboard. A rating set to null prints as unrated. A part with no `buy` prints `none`. A name that is not an id, and is a whole word of exactly one id, prints that part or that Block, so `orbe part 1n4007` prints `diode-1n4007`. When several ids share that word, the command is `unknown_part` and the hint lists those ids. `orbe part <block>` prints a Block and the command that installs it. `orbe part <id> --json` prints the card as JSON.

## The `buy` field

`buy` is optional on a part. Leave it out when there is no link. An empty list is an error. Each entry has `seller`, `url` and `checked`. `checked` is the day the link was checked, as `YYYY-MM-DD`. A buy link is an https URL: lowercase `https://`, with no userinfo, no whitespace, no query and no fragment. Userinfo is an `@` before the host. An entry has no price. A repeated entry is an error.

## Errors

| Code | What to do |
| --- | --- |
| `part_missing` | The folder `parts/<id>/` is not there, or it has no `part.jsonc`. Run the `orbe add` command in the hint. A folder with no `part.jsonc`, a file, or a link says to remove the folder the hint names first. |
| `part_exists` | A folder you named, or one the Block's scene names, is already at `parts/<id>/`, so `orbe add` wrote nothing. Remove the folders the message names and run the command in the hint. |
| `unknown_part` | No Block or part has that id. Pick one the hint names, or find one with `orbe search <words>`. |
| `registry` | The registry could not be read. The message names the URL. A network failure says to check the network and run the command again. A size or hash that does not match says the file does not match its record. |
| `not_a_project` | The current folder is not an orbe project, and a folder above it is. The hint is `orbe add -C <folder>`. |
| `scene_exists`, `file_exists` | `orbe add <block>` found another scene, or a file whose bytes differ. Nothing was written. Matching files are kept when the part folders are not there yet. Use a new folder when the bytes differ, as the hint shows. |
| `symlink` | `orbe add <block>` found a link where it would write. It does not follow the link. Remove the link, then run the command in the hint again. |
| `engine_model` | `part.jsonc` names an engine model this orbe does not have. Install the orbe the hint names. |
| `part_format` | The part file's format is not one this orbe reads. An older copy is this project's file: remove the folder and run `orbe add <id>`. A part opened by a `./` path is the user's file: update that file to the format this orbe reads, including its model, or point the scene at the part id if the file copies one. A newer part means this project needs a newer orbe. This orbe reads part formats 1, 2 and 3. Format 2 is format 1 plus `"as": "blob"` on a file param, and a blob param in a format 1 file is `scene.param`, with the hint to set `"format": 2`. Format 3 is format 2 plus an addressable LED's `DOUT` and its colour reading. A scene is format 1. |
