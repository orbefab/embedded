# Versions

Three numbers version what the engine reads and writes. This page says what each covers and which changes move it.

| Number | Where | Covers |
| --- | --- | --- |
| netlist `version` | `netlist.json`, and `netlist_format` in the manifest | which documents are valid, and what each one means |
| `run_format` | `manifest.json` | every file in the run folder, `traces/` included |
| `engine_version` | `manifest.json` | how the engine computes a run, described in [engine.md](engine.md) |

## Netlist version

A version names a set of valid documents and what each one means. A change that keeps every valid document valid, with the same meaning, keeps the version. These keep it:

- a new part kind
- a new source form or a new waveform kind
- a new analysis
- a new value of a string enum, such as a `basis`
- a rule that accepts more documents than before

Every other change bumps it. A new field on an existing object bumps it, because every field is required and an unknown field is an error.

The engine reads the versions in a list. This engine reads `[0]`. `netlist.version`'s `data.supported` is that list, and `sim doctor` reports it as `netlist_versions`. A document whose `version` is not in the list yields only `netlist.version`. The engine reads an older listed version with that version's own rules, and gives it its old meaning.

## Run format

`run_format` versions the whole folder. A reader that knows one `run_format` can read every folder written with it.

- A reader ignores a key it does not know, in every JSON object, and a file it does not know.
- A new key, a new file, a new diagnostic code and a new severity keep `run_format`.
- Removing or renaming a key, or changing a type, a unit, a meaning or a binary layout, bumps it.

`traces/index.json` keeps its own `format`. A change that bumps it also bumps `run_format`, so `run_format` alone answers whether a reader can read the folder.

`sim run` replaces a previous run of any `run_format`. [run-folder.md](run-folder.md#replacing-a-directory) says how it recognises one.

## Diagnostics

These are stable within a `run_format`:

- `code`, and the condition the code names. A condition never moves to another code.
- the shape of `refs`
- what `t_ps` means
- the keys of `data` and their types

`message` is for people. Its text can change in any version, so a reader matches on `code` and `data`, never on `message`. New codes, new severities and new `data` keys can appear without a bump, and readers accept them.

## Engine version

`engine_version` covers everything [engine.md](engine.md) describes: the step policy, how chip events are found, and the accuracy the engine reaches. A change there can move trace values within the documented accuracy without changing either format.
