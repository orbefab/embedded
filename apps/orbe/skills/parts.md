# orbe: parts

Load this for the copy in `parts/`, a version, a Block, a buy link, or an addressable LED. `orbe skills get core` covers finding a part and wiring it. `orbe part <id>` prints one part.

## The copy

`orbe add <id>…` downloads the latest part from `https://orbefab.com/orbe/r/` into `parts/<id>/`, and the parts it is built from into `parts/<id>/parts/<dep>/`. One id has one folder at the top of `parts/`. `id@version` is a usage error. The hint is the bare id. `orbe add` installs the highest version. Commit `parts/` with the scene.

```console
$ orbe add resistor-axial
wrote parts/resistor-axial 0.2.1
```

The copy is the project's. Edit it. `orbe check`, `orbe open` and `orbe test` read `parts/<id>/` as it is and never use the network. For a part of your own, put it in a folder and name that folder as `"type": "./my-resistor"`.

`orbe add` copies `part.jsonc`, the `.glb` and the anchors when the part has them. It does not copy `model.py` or `asset.json`. They build the shape.

A folder already at `parts/<id>/` is `part_exists` and nothing is written. The message names the copy's version, as `parts/led-5mm is already led-5mm 0.3.0`. A part nested inside a board does not block adding that part on its own. Adding the board again after you remove `parts/<board>/` works while that part stays at the top of `parts/`.

`@<registry>/<id>` is a usage error. This orbe reads the `@orbefab` registry only.

Addressable LEDs: `neopixel-stick-8`. Eight from three pins: `hc595-dip16`, Block `nano-shift-leds`.

5 V to 3.3 V logic: `bss138-level-shifter`.

## Blocks

`orbe add <block>` installs a Block. It downloads the parts the scene names into `parts/`, nesting each part's own parts inside that folder, then writes `scene.jsonc`, `firmware/`, `tests/` and `README.md`. It does not write `block.jsonc`. A part folder the scene names that is already there is `part_exists`. The message names those folders, and the hint is the same `orbe add <block>`. Remove them and run it again. Another `scene.jsonc` is `scene_exists`. A file with other bytes is `file_exists`. Either refusal writes nothing. A file with the Block's bytes is kept when those part folders are not there yet.

## Finding a part

`orbe add`, `orbe search` and `orbe part` need curl. When the registry cannot be reached, each prints `registry` and names the URL.

`orbe search <words>` lists parts whose id, title, maker, number, summary or tags hold every word, then Blocks whose id, title, description or categories do. An exact id comes first, then a part whose every word is a whole word of its id, title or tags, then the rest. Ids break ties. `orbe search <words> --json` puts parts in `data.parts` and Blocks in `data.blocks`. With no words, `orbe search` lists every listed part by its first category, then the Blocks. A part the registry's `unlisted` names is left out of search and of the site's list. `orbe part <id>` and `orbe add <id>` still read it. A part file's `listed` field does not decide that.

`orbe part <id>` prints the title, maker, number, license, folder hash, pins, params, ratings, what is not simulated, datasheets, where to buy it, files, the parts it is built from, and how it plugs into a breadboard. A required param prints an example. `orbe part <id> --json` prints the card. A name that is a whole word of exactly one id prints that part or Block, so `orbe part 1n4007` prints `diode-1n4007`. When several ids share the word, it is `unknown_part` and the hint lists them. `orbe part <block>` prints the Block and the command that installs it.

## The `buy` field

`buy` is optional. Leave it out when there is no link. An empty list is an error. Each entry has `seller`, `url` and `checked`. `checked` is the day the link was checked, as `YYYY-MM-DD`. A buy link is an https URL: lowercase `https://`, no userinfo (`@` before the host), no whitespace, no query and no fragment. An entry has no price. A repeated entry is an error. A buy link sells the part at its examples.

## Errors

| Code | What to do |
| --- | --- |
| `part_missing` | `parts/<id>/` is not there, or it has no `part.jsonc`. Run the `orbe add` in the hint. A folder with no `part.jsonc`, a file, or a link says to remove the folder the hint names first. |
| `part_exists` | A folder you named, or one the Block's scene names, is already at `parts/<id>/`, so `orbe add` wrote nothing. Remove the folders the message names and run the command in the hint. |
| `unknown_part` | No Block or part has that id. Pick one the hint names, or find one with `orbe search <words>`. |
| `registry` | The registry could not be read. The message names the URL. A network failure says to check the network and run the command again. A size or hash that does not match says the file does not match its record. |
| `not_a_project` | The current folder is not an orbe project, and a folder above it is. The hint is `orbe add -C <folder>`. |
| `scene_exists`, `file_exists` | `orbe add <block>` found another scene, or a file whose bytes differ. Nothing was written. Matching files are kept when the part folders are not there yet. Use a new folder when the bytes differ, as the hint shows. |
| `symlink` | `orbe add <block>` found a link where it would write. It does not follow the link. Remove the link, then run the command in the hint again. |
| `engine_model` | `part.jsonc` names an engine model this orbe does not have. Install the orbe the hint names. |
| `part_format` | The part file's format is not one this orbe reads. An older copy is this project's file: remove the folder and run `orbe add <id>`. A part opened by a `./` path is the user's file: update that file to the format this orbe reads, including its model, or point the scene at the part id if the file copies one. A newer part means this project needs a newer orbe. This orbe reads part formats 1 to 5. Format 2 adds `"as": "blob"`. Format 3 adds an addressable LED's `DOUT` and a colour reading. Format 4 adds `live`, a joint's `by`, and a capacitor's `ratings` and `series_resistance`. Format 5 adds a rating's `basis` and the model keys that came with it. A scene is format 1. |
