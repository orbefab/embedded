# Registries

A registry serves parts and Blocks as static files: an index, one record per item version, and every file by its SHA-256. This repo's `parts/` and `blocks/` are the official registry, and the catalog site is built from its records. Anyone can build a registry from a folder of their own and serve it from any static host.

## Build a registry

```sh
orbe registry build <src> <out>
```

`<src>` holds `parts/<id>/` and `blocks/<id>/`. Only folders there are items. Every file in an item's folder is one of its files, and a link anywhere is an error. `<out>` must be missing or empty. The same `<src>` always writes the same bytes.

The build fails, and writes nothing, when:

- a `part.jsonc` does not parse, has no version of the form `0.1.0`, or names another folder in `part`;
- a part `uses` a part and version the registry lacks;
- a `block.jsonc` holds a key other than `format`, `block`, `title`, `summary`, `board`, `tags` and `license`, lacks `title`, `board` or `license`, or names another folder in `block`;
- a Block's scene names a part the registry lacks, or an `@<registry>/<id>`, or does not use the Block's `board`;
- a part and a Block share a name, or an item is named `index`.

It prints the item counts and the file counts: files the records list, files stored by hash, and every file in `<out>`, which is the number a static host counts. It warns with counts of the parts that lack a summary, tags, docs, a datasheet URL or buy links, and of the Blocks that lack a summary or tags. A record leaves a missing field out. It never fills one in. `--json` prints the same as one envelope.

## Layout

```text
<out>/
  index.json              the index
  index.schema.json       JSON Schema of the index
  item.schema.json        JSON Schema of a record
  <id>@<version>.json     a part or model record
  <id>.json               a Block record
  files/<sha256>          a file's bytes, once per content
```

Both schemas are JSON Schema 2020-12, generated from the types the build writes.

## Configure a registry

A user lists registries in `registries.json`, in `$XDG_CONFIG_HOME/orbe/`, else `~/.config/orbe/`, or `%APPDATA%\orbe\` on Windows. Each name maps to a URL template:

```json
{
  "registries": {
    "@acme": "https://parts.acme.example/registry/{name}.json"
  }
}
```

A name is `@` and an id. A project names a part from that registry as `@acme/<id>`. A bare id names the official registry, which needs no line.

The template holds one variable, `{name}`. The client replaces it with an address:

| Address | Fetches |
| --- | --- |
| `index` | the index |
| `<id>@<version>` | a part or model record |
| `<id>` | a Block record |

An id is kebab case and a version is three numbers, so an address needs no escaping. A registry that `orbe registry build` wrote and that is served at `B/` has the template `B/{name}.json`.

## Index

```json
{
  "$schema": "index.schema.json",
  "items": [
    {
      "name": "led-5mm",
      "type": "part",
      "version": "0.1.0",
      "title": "5 mm LED",
      "description": "Round 5 mm through-hole LED, two leads on 2.54 mm. The longer lead is the anode. Pick its colour.",
      "categories": ["led", "light", "indicator", "through-hole"],
      "url": "led-5mm@0.1.0.json",
      "sha256": "b79a889d67892e527fdda06d9cc10dc6ba7a822f6f41782a47293c4f2d08e9f4",
      "bytes": 3628
    }
  ]
}
```

The index has one row per record, sorted by name and then by version. A Block's row has no `version`. `description` and `categories` are absent when the item has none. `url` is relative to the index's URL. `sha256` is the lowercase hex SHA-256 of the record's bytes, and `bytes` is their count. A client reads the index first. It then fetches a record from its row's `url` and checks the hash and the size before it reads the record.

## Records

Every record has these fields:

| Field | Value |
| --- | --- |
| `type` | `part`, `block` or `model` |
| `name` | the item's id |
| `version` | three numbers; a part and a model only |
| `title` | the item's title |
| `description` | the part or Block file's `summary`; absent when there is none |
| `categories` | the file's `tags`; absent when there are none |
| `license` | the item's SPDX expression |
| `registryDependencies` | items as `<id>@<version>` |
| `files` | every file in the item's folder, sorted by path |

Each entry of `files` is `{ "path", "url", "sha256", "bytes", "license" }`. `path` is the file's `/`-separated path in the item's folder. `url` is relative to the record's URL. `license` is the expression on the file's `SPDX-License-Identifier:` line within its first 40 lines, else the item's `license`. A part's `license` defaults to `MIT OR Apache-2.0`. This repo's Blocks carry `(MIT OR Apache-2.0) AND LGPL-2.1-or-later`.

**A part** adds what its page shows: `hash`, `real`, `pins`, `params`, `ratings`, `notSimulated`, `docs`, `usedBy`, `buy`, `on` and `pressable`. They hold what `orbe part --json` prints, with `not_simulated` and `used_by` written `notSimulated` and `usedBy`. `registryDependencies` is the part file's `uses`. `docs`, `buy` and `on` are absent when the part has none, and a doc with no `url` is a name with no link. `orbe add` writes each file to `parts/<id>/<path>`.

**A Block** adds `board`, the part its firmware runs on. `registryDependencies` lists every part the scene runs, a board's own parts included, at the versions in the same build. Those are the versions its tests ran against. `orbe add` writes each file except `block.jsonc` and `README.md` to the project at its `path`, then copies its parts.

**A model** is an engine model shipped as WebAssembly. It has only the common fields. Its own fields come with the device contract.

## Hashing

- A file's SHA-256 is over its bytes, in lowercase hex. The file is stored once at `files/<sha256>`, however many records list it.
- A part's `hash` is the folder hash that `parts.lock` records. It is the SHA-256 of one line `<sha256>  <path>\n` per file, sorted by path, written as `sha256:<hex>`. A client that copies a part's files computes it again and compares.
- A record's SHA-256 is in its index row.

## Versions

- A part or model record at `<id>@<version>` never changes once it is served. A change to the part is a new version.
- A Block has no version. Each build replaces its record.
- A build serves the versions its folder holds, one per id.

## Which registry a part came from

A `parts.lock` line is `<id> <version> <registry> sha256:<hash>`. A project takes each part id from one registry, so `parts.lock` has at most one line per id. Two registries can both serve `led-5mm`, and a project holds one of the two. `orbe check` reports a second line for an id as `scene.duplicate`, and `orbe add` refuses to run until the line is gone or `orbe add --relock` rebuilds the lock.

`<registry>` is the registry's URL template, exactly as `registries.json` maps it, so a lock reads the same on every machine. The official registry's value is `orbe_scene::REGISTRY`.

## Not in this version

- `orbe add @<registry>/<id>` from a registry URL: read `registries.json`, the index and the record, check every hash, then write the copy and its lock line.
- `orbe search --refresh`: search registry indexes instead of the catalog built into `orbe`.
- The official registry's URL template. It is set with the site's domain, and `REGISTRY` changes to it.
- Old versions in a build: carry a previous build's records and files forward, and refuse a record that changes at a version already served.
- Model items: a source folder for them and their fields come with the device contract.
- Type-checking in the build: today it checks syntax, names, versions and dependencies, and `orbe check` checks the rest.
- A part or Block that uses a part from another registry.
