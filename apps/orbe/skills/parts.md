# orbe: parts

Load this for the copy in `parts/`, a version, a Block, a buy link, or an addressable LED. `orbe skills get core` covers finding a part and wiring it. `orbe part <id>` prints one part.

## The copy

`orbe add <id>…` downloads the latest part from `https://orbefab.com/orbe/r/` into `parts/<id>/`, and the parts it is built from into `parts/<id>/parts/<dep>/`. One id has one folder at the top of `parts/`. `id@version` is a usage error. The hint is the bare id. `orbe add` installs the highest version. Commit `parts/` with the scene.

```console
$ orbe add resistor-axial
wrote parts/resistor-axial 0.2.1
```

The copy is the project's. Edit it. `orbe check`, `orbe open` and `orbe test` read `parts/<id>/` as it is and never use the network. For a part of your own, put it in a folder and name that folder as `"type": "./my-resistor"`.

`orbe add` copies `part.jsonc`, the `.glb` and the anchors when the part has them. It does not copy `model.py` or `asset.json`.

A folder already at `parts/<id>/` is `part_exists` and nothing is written. The message names the copy's version, as `parts/led-5mm is already led-5mm 0.3.1`. A part nested inside a board does not block adding that part on its own. Adding the board again after you remove `parts/<board>/` works while that part stays at the top of `parts/`.

`@<registry>/<id>` is a usage error. This orbe reads the `@orbefab` registry only.

Addressable LEDs: `neopixel-stick-8`, `neopixel-ring-16` (`nano-neopixel-ring`). 4-pin RGB LEDs: `led-5mm-rgb-cc` (`nano-rgb-led`), `led-5mm-rgb-ca` (`@<id>.red`/`.green`/`.blue`). Shift register: `hc595-dip16` (SRCLR high, OE low; `nano-shift-leds`). Level shifter: `bss138-level-shifter`.

Temperature sensors: `tmp36` (`nano-tmp36`), `thermistor-ntc-10k` (`nano-thermistor`). Batteries: `battery-9v` (`nano-9v-regulator`), `battery-aa-4x` (`nano-aa-pack`). 2-axis joystick: `joystick-ky023` (`@<id>.x.position`, `@<id>.y.position`, `@<id>.sw`; `nano-joystick`).

## Blocks

`orbe add <block>` installs a Block: downloads its scene parts into `parts/` (nesting dependencies), then writes `scene.jsonc`, `firmware/`, `tests/` and `README.md` (not `block.jsonc`). Existing part folders fail with `part_exists`; another `scene.jsonc` fails with `scene_exists`; a differing file fails with `file_exists`. Each refusal writes nothing. Matching files stay when the part folders are absent.

## Finding a part

`orbe add`, `orbe search` and `orbe part` need curl; unreachable registries fail with `registry` and the URL.

`orbe search <words>` matches parts (id, title, maker, number, summary, tags) then Blocks (id, title, description, categories): exact id first, then whole-word id/title/tag hits, then the rest, ties by id (`--json` puts them in `data.parts` and `data.blocks`). With no words it lists every listed part by first category, then Blocks. Parts in the registry's `unlisted` are omitted from search and the site, though `orbe part` and `orbe add` still read them.

`orbe part <id>` prints title, maker, number, license, hash, pins, params, ratings, not simulated, datasheets, buy links, files, uses, and breadboard seating (`--json` prints the card). A unique id word matches that part or Block (`orbe part 1n4007` prints `diode-1n4007`); several return `unknown_part` listing them. `orbe part <block>` prints the Block and its install command.

## The `buy` field

`buy` is optional; omit when there is no link (an empty list is an error). Each entry has `seller`, `url` (lowercase `https://`, no userinfo, whitespace, query or fragment) and `checked` (`YYYY-MM-DD`), with no price and no repeats.

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
| `part_format` | The part file's format is not one this orbe reads. An older copy is this project's file: remove the folder and run `orbe add <id>`. A part opened by a `./` path is the user's file: update that file to the format this orbe reads, including its model, or point the scene at the part id if the file copies one. A newer part means this project needs a newer orbe. This orbe reads part formats 1 to 7. Format 2 adds `"as": "blob"`. Format 3 adds an addressable LED's `DOUT` and a colour reading. Format 4 adds `live`, a joint's `by`, and a capacitor's `ratings` and `series_resistance`. Format 5 adds a rating's `basis`. Format 6 adds a servo angle and a screen. Format 7 adds Steinhart-Hart curves, resistor max voltage, and `characterized` ratings. A scene is format 1. |
