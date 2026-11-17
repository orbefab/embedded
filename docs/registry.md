# Registries

A registry serves parts and Blocks as static files: an index, one record per item version, and every file by its SHA-256. This repo's `parts/` and `blocks/` are the official registry, `@orbefab`, and the catalog site is built from its records. Anyone can build a registry from a folder of their own and serve it from any static host. This page is the contract a client reads a registry by.

## Build a registry

```sh
orbe registry build <src> <out> [--previous <dir>]
```

`<src>` holds `registry.jsonc`, `parts/<id>/` and `blocks/<id>/`. `registry.jsonc` names the registry:

```jsonc
{
  "format": 1,
  "name": "@acme",
  "homepage": "https://parts.acme.example",
}
```

`name` is `@` and an id. `homepage` is an `https://` URL and may be left out. Only folders in `parts/` and `blocks/` are items. Every published file in an item's folder is one of its files, and a link among those files is an error. `<out>` must be missing or empty. The build writes into a sibling folder and renames it to `<out>` once every file is written, so a failed build leaves nothing to serve. The same `<src>` always writes the same bytes.

The build fails, and writes nothing, when:

- `registry.jsonc` is missing, holds a key other than `format`, `name` and `homepage`, or has a bad `name` or `homepage`;
- an item folder's name is not an id, or is reserved (see [Names](#names));
- a file path in an item is one some platform cannot hold (see step 5 of [Verify what you fetch](#verify-what-you-fetch)), or two paths differ only in case;
- a `part.jsonc` does not parse, has no version in the one spelling, has no `license`, or names another folder in `part`;
- a part `uses` a part and version the registry lacks;
- a `block.jsonc` holds a key other than `format`, `block`, `version`, `title`, `summary`, `board`, `tags` and `license`, lacks `version`, `title`, `board` or `license`, or names another folder in `block`;
- a Block's scene names a part the registry lacks, or an `@<registry>/<id>`, or does not use the Block's `board`;
- a part and a Block share a name;
- with `--previous`, the earlier build does not check out, or a version it serves would build to a different record (see [Versions](#versions)).

It prints the item counts and the file counts: files the records list, files stored by hash, and every file in `<out>`, which is the number a static host counts. It warns with counts of the parts that lack a summary, tags, docs, a datasheet URL or buy links, and of the Blocks that lack a summary or tags. It also names, per item, each hidden file (a path segment that starts with `.`) and each file over 1 MB, since a file in an item's folder is published, apart from build output, and cached by its hash. `--json` prints the same as one envelope.

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
- **A dependency** is `[@<registry>/]<id>@<version>`, such as `led-5mm@0.1.0` or `@acme/lamp@1.2.0`. With no `@<registry>/` it is in the same registry as the record that names it. A parser takes off a leading `@<registry>/` first, then splits the rest at its one `@`. The build writes only the bare form today.

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

A project names a part from that registry as `@acme/<id>`. A bare id names `@orbefab`, which needs no line. Its template is `https://orbefab.com/embedded/r/{name}.json`. Nothing fetches from a registry yet.

A template is an `https://` URL with exactly one `{name}`. `{name}` starts its last path segment and is followed only by a fixed suffix, such as `.json`. A template has no query and no fragment. A client replaces `{name}` with `index`, literally, to get the index's URL. It never builds another URL from the template. It takes each record's URL from its index row and each file's URL from its record, and resolves both against the URL of the document that holds them. A registry that `orbe registry build` wrote and that is served at `B/` has the template `B/{name}.json`.

## Index

```json
{
  "$schema": "index.schema.json",
  "format": 1,
  "name": "@orbefab",
  "homepage": "https://orbefab.com/embedded",
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

`name` is the registry's name and `homepage` its page, absent when `registry.jsonc` has none. `items` has one row per record, sorted by name and then by version, oldest first. Every row has a `version`. `description` and `categories` are absent when the item has none. `url` is `<name>@<version>.json`, relative to the index's URL. `sha256` is the lowercase hex SHA-256 of the record's bytes, and `bytes` is their count.

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

This repo's parts carry `MIT OR Apache-2.0`, every file of them. A Block is `(MIT OR Apache-2.0) AND LGPL-2.1-or-later` as a whole, and its `REUSE.toml` gives each file its own: its firmware images and `firmware/LICENSE-runtime` are `LGPL-2.1-or-later`, `firmware/LICENSE-libc` is `BSD-3-Clause`, and every other file is `MIT OR Apache-2.0`.

**A part** adds what its page shows: `real`, `pins`, `params`, `ratings`, `notSimulated`, `docs`, `buy`, `lit`, `dim`, `pressable` and `holes`. `lit` and `dim` are an LED's brightness thresholds, each `{ "of", "above", "unit" }`. `holes` is a board's, such as a breadboard's: `depth` in mm, and `strips`, the holes each strip joins, each of them also in `pins`. `docs`, `buy`, `lit`, `dim` and `holes` are absent when the part has none, `pressable` is absent when false, and a doc with no `url` is a name with no link. `orbe add` writes each file to `parts/<id>/<path>`.

A part record is what `orbe part --json` prints, changed in exactly these ways:

- `type`, `$schema` and `format` are added.
- `uses`, an object from id to version, becomes `registryDependencies`, an array of `<id>@<version>` in id order.
- Each entry of `files` gains `url`.
- `not_simulated` is written `notSimulated`.
- `used_by` is left out. A served record never changes, and the parts that use a part grow with the registry. A client finds them in the other records' `registryDependencies`.

Every other field keeps its name and its value. `orbe part --json` stays snake case. A test converts every catalog part both ways under this mapping.

**A Block** adds `board`, the part its firmware runs on. `registryDependencies` lists every part the scene runs, a board's own parts included, at the versions those parts have in the same build. The build checks that each one is there. It does not run the Block's tests. In this repo, `scripts/check.sh` runs every Block's tests against the parts of the same commit, which is what the official registry is built from. `orbe add` writes each file except `block.jsonc` and `README.md` to the project at its `path`, then copies its parts.

### Absent fields and null

A field with no value is left out, never written as `null`. `null` is a value in two places only: a rating's `value`, where it means unrated, and a param's `default`, where the part file's default is `null`. The schemas allow `null` there and nowhere else.

### Format and fields a client does not know

The index and every record carry `"format": 1`. A client refuses a format it does not know and ignores fields it does not know. A new optional field keeps the format. Removing a field, or changing a field's type or meaning, raises it. The schemas allow fields they do not name, so a record with a newer optional field still validates.

In the index and in a record, `$schema` names the schema file beside it, for editors. In the two schema files, `$schema` is JSON Schema's own keyword and names the dialect, Draft 2020-12.

## Hashing

- A file's SHA-256 is over its bytes, in lowercase hex. The file is stored once at `files/<sha256>`, however many records list it.
- A record's `hash` is the SHA-256 of one line `<sha256>  <path>\n` per entry of `files`, sorted by path, written as `sha256:<hex>`. A part's is the hash `parts.lock` records.
- Build output in the folder is not one of those files. A `__pycache__` directory, a `.pyc` file, and a `.pytest_cache` or `.venv` directory are left out of `files`, out of the folder hash, and out of what `orbe add` copies. Any other file counts, including one an author added beside `part.jsonc`. The names are fixed. A project has no copy of this repo's `.gitignore`.
- A record's SHA-256 is in its index row.

## Verify what you fetch

TLS to the registry's host is the trust root. The index is not signed. Its hashes tie each record to the index, and each record's hashes tie its files to it. A client that installs an item does this:

1. Fetch the index from the template with `{name}` replaced by `index`. Refuse a `format` it does not know. Reject the index if two rows share a name and version.
2. Pick the row. A lock line pins its version. Otherwise take the highest version of the name. Require the row's `url` to be `<name>@<version>.json`.
3. Fetch the record from the row's `url`, resolved against the index's URL. Verify its size and SHA-256 against the row before parsing it.
4. Require the record's `name`, `type` and `version` to equal the row's, and its `format` to be one it knows.
5. Reject the record if two entries of `files` share a path, or if a path is empty or absolute, or holds an empty, `.` or `..` segment, a backslash, a control character or one of `:*?"<>|`, a segment that ends in a dot or a space, or a segment whose name before its first dot is a reserved device name. Reject two paths that are equal after case folding.
6. Require each file's `url` to be exactly `files/<sha256>`, with that entry's own `sha256`. Fetch it, resolved against the record's URL, and check its size and SHA-256.
7. Compute the folder hash over the files and require it to equal the record's `hash`.
8. Write only after every check passes.

## Versions

- A record at `<id>@<version>` never changes once it is served. A change to the item is a new version.
- Versions order by number: by the first, then the second, then the third. `0.10.0` is newer than `0.9.0`. The latest version of an item is the highest.
- `released.lock`, at the root of this repo, records each part and Block a release served: `<id> <version> sha256:<hash>`, one line, sorted by id. The hash is the SHA-256 of the record `orbe registry build` writes for `<id>@<version>`, the same digest that record's index row stores, written with `sha256:` in front. The check rebuilds each record from source and compares it to that hash, so it refuses a tree `--previous` would refuse. The line does not also record the folder hash. That hash is a field of the record, so this digest covers it, and a folder hash cannot see the part versions a Block's record names. The file changes only when a release is tagged. Until the first tag it is empty, because nothing has been served. A missing line means that item is not released yet. A line, once written, stays. `scripts/released.sh` keeps a line whose folder is gone, adds a line for a new item, and advances a line to a newer version. It never moves a version backwards. An item put back at a recorded version must build the same record. `scripts/check.sh fast` fails when it does not. The message names the item and says to bump its version. A folder at a newer version than its line passes. Run `scripts/released.sh` when tagging a release, and commit the file with the tag.
- A part's `format` covers its `model`. A netlist change that an existing part file cannot survive raises the part `format`.
- A registry keeps serving every version it ever served, so every `parts.lock` that pins one can still be filled. `--previous <dir>` makes the build keep that rule. It reads the build in `<dir>`, checks each record and file against its hashes, and carries forward every record the source no longer holds, with its files. A version the source still holds must build to the same bytes as the record `<dir>` serves, or the build refuses and asks for a new version. A binary built from a part whose bytes changed without a new version still carries that `id@version`. `orbe check` then reports `part_modified` for a sealed copy of that version that matches its lock, and names the field that differs.
- A Block record names the versions of its parts, so a Block whose parts get new versions takes a new version too.

## Caching

`files/<sha256>` and `<id>@<version>.json` never change, so a client or a CDN caches them forever. `index.json`, `item.schema.json` and `index.schema.json` change in place at the same URL, so a client revalidates them on each use.

## Which registry a part came from

A `parts.lock` line is `<id> <version> @<registry> sha256:<hash>`. `@<registry>` is the registry's name, and the parts of the catalog built into `orbe` lock as `@orbefab`. A client maps the name to a template through `registries.json` when it fetches, so a lock reads the same on every machine and a registry that moves changes one line of config, never a lock. A line whose registry field is not `@` and an id is an error, and `orbe add --relock` rebuilds the lock. A folder that matches its line and is newer than the part this orbe carries keeps that line.

A project takes each part id from one registry, so `parts.lock` has at most one line per id. Two registries can both serve `led-5mm`, and a project holds one of the two. `orbe check` reports a second line for an id as `scene.duplicate`, and `orbe add` refuses to run until the line is gone or `orbe add --relock` rebuilds the lock.

## Not in this version

- `orbe add @<registry>/<id>` from a registry: read `registries.json`, then follow [Verify what you fetch](#verify-what-you-fetch) and write the copy and its lock line.
- `orbe search --refresh`: search registry indexes instead of the catalog built into `orbe`.
- Type-checking and Block tests in the build: today it checks syntax, names, versions, licences and dependencies, and `orbe check` and `orbe test` check the rest.
- A part or Block that uses a part from another registry. The dependency grammar has room for it, and the build refuses it today.
