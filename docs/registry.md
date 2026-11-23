# Registries

A registry serves parts and Blocks as static files: an index, one record per item version, and every file by its SHA-256. This repo's `parts/` and `blocks/` are the official registry, `@orbefab`, and the catalog site is built from its records. Anyone can build a registry from a folder of their own and serve it from any static host. This page is the contract a client reads a registry by.

## Build a registry

```sh
orbe registry build <src> <out>
```

`<src>` holds `registry.jsonc`, `parts/<id>/` and `blocks/<id>/`. `registry.jsonc` names the registry:

```jsonc
{
  "format": 1,
  "name": "@acme",
  "homepage": "https://parts.acme.example",
  "unlisted": ["soic-chip"],
}
```

`name` is `@` and an id. `homepage` is an `https://` URL and may be left out. `unlisted` names the part ids that `orbe search` and the site's parts list leave out, and may be left out. An id in it that is not a part in the tree fails the build, and the message names that id. A part file's own `listed` field is ignored, so hiding or showing a part does not change its record and does not need a new version. Only folders in `parts/` and `blocks/` are items. Every published file in an item's folder is one of its files, and a link among those files is an error. `<out>` must be missing or empty. The build writes into a sibling folder and renames it to `<out>` once every file is written, so a failed build leaves nothing to serve. The same `<src>` always writes the same bytes.

The build fails, and writes nothing, when:

- `registry.jsonc` is missing, holds a key other than `format`, `name`, `homepage` and `unlisted`, has a bad `name` or `homepage`, or names in `unlisted` an id that is not a part in the tree;
- an item folder's name is not an id, or is reserved (see [Names](#names));
- a file path in an item is one some platform cannot hold (see step 5 of [Verify what you fetch](#verify-what-you-fetch)), or two paths differ only in case;
- a `part.jsonc` does not parse, the part-file reader refuses it, it has no version in the one spelling, it has no `license`, or it names another folder in `part`. The reader is the one `orbe check` uses, and the finding has the same code, place and message;
- a part `uses` a part and version the registry lacks;
- a `block.jsonc` holds a key other than `format`, `block`, `version`, `title`, `summary`, `board`, `tags` and `license`, lacks `version`, `title`, `board` or `license`, or names another folder in `block`;
- a Block's `scene.jsonc` does not parse or the scene reader refuses it, with that same finding, or the scene names a part the registry lacks, or an `@<registry>/<id>`, or does not use the Block's `board`;
- a file param names a path outside the folder of the item that writes it: an absolute path, or a `..` that leaves `parts/<id>/` for a part's default or a part file's value, or leaves the Block's folder for a Block's scene. An install copies only that folder, so the installed copy would not have the file. This is `registry_invalid`, and it names the file, the place and the path. `orbe check` in the author's own tree reads such a path when the file happens to be there, so it can pass there and still be refused here;
- a param's `example` sits beside a `default`, is out of range, is not a JSON number on a number param, or names a file outside the part's folder. `example` is a value a scene could set, on a param with no `default`. A number example is a JSON number. The finding is the part-file reader's, with the same code, place and message;
- a required param has no `example`. The finding names that param's key in `parts/<id>/part.jsonc` and says to add one;
- installing the part or the Block and running `orbe check` would refuse it, as [What the build checks](#what-the-build-checks) says. The finding has the same code, place and message;
- a part and a Block share a name.

It prints the item counts and the file counts: files the records list, files stored by hash, and every file in `<out>`, which is the number a static host counts. It warns with counts of the parts that lack a summary, tags, docs, a datasheet URL or buy links, and of the Blocks that lack a summary or tags. It also names, per item, each hidden file (a path segment that starts with `.`) and each file over 1 MB, since a file in an item's folder is published, apart from build output and the model inputs `model.py` and `asset.json`, and cached by its hash. `--json` prints the same as one envelope.

### What the build checks

The build reads each Block's scene the way `orbe check` reads it after a project installs the Block: the project holds the registry's part folders and the Block's own files. Every finding `orbe check` would print refuses the Block, in `blocks/<id>/scene.jsonc`.

A part has no scene of its own. The build places it alone as `r1` in a project that holds the registry's part folders and reads that scene the way `orbe check` reads a scene. Each required param is set to its `example`. A required param with none is refused at that param's key in `parts/<id>/part.jsonc`. An example that names a file names the copy already in the nested part folder. When the part names no ground, and no part inside it does, the scene adds a voltage source and names one of its pins as ground. When a part in that tree names a ground, those grounds must agree, as they must in a scene that only places the part. Every finding refuses the part. A finding names `parts/<id>/part.jsonc`, or the file it is in. A finding about the grounds names `scene.jsonc`, and so does a finding about a value the build set from an example, because that scene exists only in memory.

## Layout

```text
<out>/
  index.json              the index
  index.schema.json       JSON Schema of the index
  item.schema.json        JSON Schema of a record
  <id>@<version>.json     a record
  files/<sha256>          a file's bytes, once per content
```

Both schemas are JSON Schema Draft 2020-12, generated from the types the build writes. A test validates every record and the index against them.

## Names

- **An id** is kebab case: a lowercase ASCII letter, then lowercase ASCII letters, digits and `-`, at most 64 characters in all. These ids are reserved in every registry, and no item takes one: `index`, and the names Windows keeps for devices, `con`, `prn`, `aux`, `nul`, `com0` to `com9` and `lpt0` to `lpt9`. An id has no `.`, so no record can take the name of `index.schema.json` or `item.schema.json`.
- **A version** is three numbers separated by `.`. Each is ASCII digits with no sign and no leading zero, so `0` is a number and `01` is not. One version has one spelling.
- **A registry name** is `@` and an id, such as `@orbefab`.
- **A dependency** is `[@<registry>/]<id>@<version>`, such as `led-5mm@1.2.3` or `@acme/lamp@1.2.0`. With no `@<registry>/` it is in the same registry as the record that names it. A parser takes off a leading `@<registry>/` first, then splits the rest at its one `@`. The build writes only the bare form today.

Ids, versions, `@` and `.` are safe in a URL path segment and in a file name on every platform, so nothing in this contract is percent-encoded.

The schemas carry these rules as patterns.

## Configure a registry

A user lists registries in `registries.json`, in `$XDG_CONFIG_HOME/orbe/`, else `~/.config/orbe/`, or `%APPDATA%\orbe\` on Windows. Each registry name maps to a URL template:

```json
{
  "registries": {
    "@acme": "https://parts.acme.example/registry/{name}.json"
  }
}
```

A project names a part from that registry as `@acme/<id>`. A bare id names `@orbefab`, which needs no line. Its template is `https://orbefab.com/orbe/r/{name}.json`. This `orbe` does not read `registries.json`. It fetches `@orbefab` from that template. `orbe add`, `orbe search`, `orbe part` and `orbe blocks` are the commands that fetch. `orbe check`, `orbe open`, `orbe test` and `orbe show` do not.

A template is an `https://` URL with exactly one `{name}`. `{name}` starts its last path segment and is followed only by a fixed suffix, such as `.json`. A template has no query and no fragment. A client replaces `{name}` with `index`, literally, to get the index's URL. It never builds another URL from the template. It takes each record's URL from its index row and each file's URL from its record, and resolves both against the URL of the document that holds them. A registry that `orbe registry build` wrote and that is served at `B/` has the template `B/{name}.json`.

## Index

```json
{
  "$schema": "index.schema.json",
  "format": 1,
  "name": "@orbefab",
  "homepage": "https://orbefab.com/orbe",
  "items": [
    {
      "name": "led-5mm",
      "type": "part",
      "version": "0.1.0",
      "title": "5 mm LED",
      "description": "Round 5 mm through-hole LED, two leads on 2.54 mm. The longer lead is the anode. Pick its colour.",
      "categories": ["led", "light", "indicator", "through-hole"],
      "url": "led-5mm@0.1.0.json",
      "sha256": "28b7ae2beb1a39ba0444667b0208f03887f4aec3fb01e81e6ebd9382986a2b20",
      "bytes": 3832
    }
  ]
}
```

`name` is the registry's name and `homepage` its page, absent when `registry.jsonc` has none. `items` has one row per record, sorted by name and then by version, oldest first. Every row has a `version`. `description` and `categories` are absent when the item has none. A part row may also carry `maker` and `number`, from the part file's `real` fields, and leaves a field out when the part names none. A Block row carries neither. A part row carries `"listed": false` when `registry.jsonc`'s `unlisted` names that part, which is what search and the parts list leave out, and leaves `listed` out when the part is offered. A part file's own `listed` field is ignored. `url` is `<name>@<version>.json`, relative to the index's URL. `sha256` is the lowercase hex SHA-256 of the record's bytes, and `bytes` is their count.

## Records

Every record has these fields:

| Field | Value |
| --- | --- |
| `type` | `part` or `block` |
| `$schema` | `item.schema.json` |
| `format` | `1` |
| `name` | the item's id |
| `version` | the item's version |
| `title` | the item's title |
| `description` | the item file's `summary`; absent when there is none |
| `categories` | the item file's `tags`; absent when there are none |
| `license` | the item's SPDX expression |
| `registryDependencies` | the items it is built from or runs, as dependencies |
| `files` | every published file in the item's folder, sorted by path |
| `hash` | the folder hash over `files`, as `sha256:<hex>` |

Each entry of `files` is `{ "path", "url", "sha256", "bytes", "license" }`. `path` is the file's `/`-separated path in the item's folder. `url` is `files/<sha256>`, relative to the record's URL. `license` is the file's licence as [REUSE 3.3](https://reuse.software/spec-3.3/) reads it from the item's folder. When the folder holds a `REUSE.toml`, the last of its `[[annotations]]` whose `path` matches the file gives the licence, weighed against the file's own `SPDX-License-Identifier:` line by the annotation's `precedence`: `closest`, the default, keeps the file's own line, `override` keeps the annotation's, and `aggregate` joins both with `AND`. A file no annotation matches takes the expression on its `SPDX-License-Identifier:` line within its first 40 lines, else the item's `license`. A `REUSE.toml` that does not read fails the build. A file that cannot carry a line of its own, such as a firmware image or a licence text, gets its licence this way. A part names its `license` in `part.jsonc`, and the build refuses a part that does not, so no record carries a licence its author did not write.

This repo's parts carry `MIT OR Apache-2.0`, every file of them, except the ESP32-S3 chip. In `parts/esp32s3-qfn56`, `REUSE.toml` gives `esp32s3_rev0_rom.elf`, `LICENSE-rom` and `NOTICE` `Apache-2.0` only. `hello_world.bin`, `partition-table.bin` and `NOTICE-hello` are `Apache-2.0 AND MIT AND BSD-3-Clause AND BSD-2-Clause AND LicenseRef-Cadence-Xtensa AND LicenseRef-newlib AND LicenseRef-GCC-Runtime-Library-Exception`. `LICENSE-freertos` is `MIT`, `LICENSE-tlsf` is `BSD-3-Clause`, `LICENSE-ubsan` is `BSD-2-Clause`, `LICENSE-xtensa` is `LicenseRef-Cadence-Xtensa` and `LICENSE-newlib` is `LicenseRef-newlib`. That part as a whole is `(MIT OR Apache-2.0) AND Apache-2.0 AND MIT AND BSD-3-Clause AND BSD-2-Clause AND LicenseRef-Cadence-Xtensa AND LicenseRef-newlib AND LicenseRef-GCC-Runtime-Library-Exception`. A Block is `(MIT OR Apache-2.0) AND LGPL-2.1-or-later` as a whole, except `nano-neopixel-stick`, whose image also links Adafruit NeoPixel 1.15.5 and so is `(MIT OR Apache-2.0) AND LGPL-2.1-or-later AND LGPL-3.0-or-later`. No Block file carries its own `SPDX-License-Identifier:` line, so each file's record takes that expression. `firmware/NOTICE` states which file carries which licence: the firmware image and `firmware/LICENSE-runtime` are `LGPL-2.1-or-later`, `firmware/LICENSE-libc` is `BSD-3-Clause`, and the Block's own files are `MIT OR Apache-2.0`. `nano-neopixel-stick` also names `firmware/LICENSE-neopixel` as `LGPL-3.0-or-later`, and its image carries that licence as well as the runtime's.

**A part** adds what its page shows: `real`, `pins`, `params`, `ratings`, `notSimulated`, `docs`, `buy`, `lit`, `dim`, `pressable` and `holes`. `lit` and `dim` are an LED's brightness thresholds, each `{ "of", "above", "unit" }`. A choice in `params` carries its own `lit` or `dim` when it sets that threshold. `holes` is a board's, such as a breadboard's: `depth` in mm, and `strips`, the holes each strip joins, each of them also in `pins`. `docs`, `buy`, `lit`, `dim` and `holes` are absent when the part has none, `pressable` is absent when false, and a doc with no `url` is a name with no link. `orbe add` writes each file to `parts/<id>/<path>`.

A part record is what `orbe part --json` prints, changed in exactly these ways:

- `type`, `$schema` and `format` are added.
- `uses`, an object from id to version, becomes `registryDependencies`, an array of `<id>@<version>` in id order.
- Each entry of `files` gains `url`.
- `not_simulated` is written `notSimulated`.
- `used_by` is left out. A served record never changes, and the parts that use a part grow with the registry. A client finds them in the other records' `registryDependencies`.

Every other field keeps its name and its value. `orbe part --json` stays snake case. A test converts every catalog part both ways under this mapping.

**A Block** adds `board`, the part its firmware runs on. `registryDependencies` lists every part the scene runs, a board's own parts included, at the versions those parts have in the same build. The build checks that each one is there. It does not run the Block's tests. In this repo, `scripts/check.sh` runs every Block's tests against the parts of the same commit, which is what the official registry is built from. `orbe add` writes each file except `block.jsonc` to the project at its `path`, including `README.md`, then downloads the parts its scene names into `parts/<id>/`.

### Absent fields and null

A field with no value is left out, never written as `null`. `null` is a value in two places only: a rating's `value`, where it means unrated, and a param's `default`, where the part file's default is `null`. The schemas allow `null` there and nowhere else.

### Format and fields a client does not know

The index and every record carry `"format": 1`. A client refuses a format it does not know and ignores fields it does not know. A new optional field keeps the format. Removing a field, or changing a field's type or meaning, raises it. The schemas allow fields they do not name, so a record with a newer optional field still validates.

In the index and in a record, `$schema` names the schema file beside it, for editors. In the two schema files, `$schema` is JSON Schema's own keyword and names the dialect, Draft 2020-12.

## Hashing

- A file's SHA-256 is over its bytes, in lowercase hex. The file is stored once at `files/<sha256>`, however many records list it.
- A record's `hash` is the SHA-256 of one line `<sha256>  <path>\n` per entry of `files`, sorted by path, written as `sha256:<hex>`. A part's is the folder hash over those files.
- Build output in the folder is not one of those files, and neither are `model.py` and `asset.json`, the inputs a part's 3D model is built from. The GLB still ships. A `__pycache__` directory, a `.pyc` file, and a `.pytest_cache` or `.venv` directory are left out of `files`, out of the folder hash, and out of what `orbe add` copies, and so are those two names, at any depth. Any other file counts, including one an author added beside `part.jsonc`. The names are fixed. A project has no copy of this repo's `.gitignore`.
- A record's SHA-256 is in its index row.

## Verify what you fetch

TLS to the registry's host is the trust root. The index is not signed. Its hashes tie each record to the index, and each record's hashes tie its files to it. A client that installs an item does this:

1. Fetch the index from the template with `{name}` replaced by `index`. Refuse a `format` it does not know. Reject the index if two rows share a name and version.
2. Pick the row. Take the highest version of the name. `orbe add <id>@<version>` is a usage error, and the hint is the bare id. Require the row's `url` to be `<name>@<version>.json`.
3. Fetch the record from the row's `url`, resolved against the index's URL. Verify its size and SHA-256 against the row before parsing it.
4. Require the record's `name`, `type` and `version` to equal the row's, and its `format` to be one it knows.
5. Reject the record if two entries of `files` share a path, or if a path is empty or absolute, or holds an empty, `.` or `..` segment, a backslash, a control character or one of `:*?"<>|`, a segment that ends in a dot or a space, or a segment whose name before its first dot is a reserved device name. Reject two paths that are equal after case folding.
6. Require each file's `url` to be exactly `files/<sha256>`, with that entry's own `sha256`. Fetch it, resolved against the record's URL, and check its size and SHA-256.
7. Compute the folder hash over the files and require it to equal the record's `hash`.
8. Write only after every check passes.

## Versions

- A record at `<id>@<version>` never changes once it is served. A change to the item is a new version.
- Versions order by number: by the first, then the second, then the third. `0.10.0` is newer than `0.9.0`. The latest version of an item is the highest.
- `released.lock`, at the root of this repo, records each part and Block a release served: `<id> <version> sha256:<hash>`, one line, sorted by id. The hash is the SHA-256 of the record `orbe registry build` writes for `<id>@<version>`, the same digest that record's index row stores, written with `sha256:` in front. The check rebuilds each record from source and compares it to that hash. The line does not also record the folder hash. That hash is a field of the record, so this digest covers it, and a folder hash cannot see the part versions a Block's record names. The file changes only when a release is tagged. Until the first tag it is empty, because nothing has been served. A missing line means that item is not released yet. A line, once written, stays. `scripts/released.sh` keeps a line whose folder is gone, adds a line for a new item, and advances a line to a newer version. It never moves a version backwards. An item put back at a recorded version must build the same record. `scripts/check.sh fast` fails when it does not. The message names the item and says to bump its version. A folder at a newer version than its line passes. Run `scripts/released.sh` when tagging a release, and commit the file with the tag.
- A part's `format` covers its `model`. A netlist change that an existing part file cannot survive raises the part `format`.
- The site serves the versions in this repo. A version that has left the repo is not republished.
- A Block record names the versions of its parts, so a Block whose parts get new versions takes a new version too.

## Caching

`files/<sha256>` and `<id>@<version>.json` never change, so a client or a CDN caches them forever. `index.json`, `item.schema.json` and `index.schema.json` change in place at the same URL, so a client revalidates them on each use.

## Where a copy comes from

`orbe add <id>` downloads the latest version of that part or Block from `@orbefab` and writes the published files into the project. A part goes to `parts/<id>/`, and the parts it is built from go to `parts/<id>/parts/<dep>/`. `model.py` and `asset.json` are not in that copy. The GLB is. A Block writes its scene, firmware, tests and README, and the parts its scene names, the same way. The project owns those copies. `orbe` reads them as they are and does not compare them to the registry. `@<registry>/<id>` is refused: this orbe reads `@orbefab` only. `orbe check`, `orbe open`, `orbe test` and `orbe show` never use the network.

## Not in this version

- Running a Block's tests during the build. `orbe test` checks those. The build runs the same check as `orbe check` on every Block scene, and on every part at its examples, as [What the build checks](#what-the-build-checks) says.
- A part or Block that uses a part from another registry. The dependency grammar has room for it, and the build refuses it today.
