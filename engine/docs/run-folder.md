# Run folder

`sim run <netlist.json> --out <dir>` writes four files. A rejected netlist writes the same four files.

```text
<dir>/
  manifest.json
  netlist.json
  diagnostics.json
  observations.json
```

Each file is pretty JSON with a two-space indent and a trailing newline. `manifest.json` uses the field order in the example below. Keys inside a diagnostic `data` object are alphabetical. Observation keys follow ref order.

## manifest.json

```json
{
  "run_format": 0,
  "netlist_format": 0,
  "engine_version": "0.1.0",
  "input": {
    "file": "netlist.json",
    "sha256": "<lowercase hex sha256 of the input bytes>",
    "bytes": 902
  },
  "seed": 0,
  "created_at": "2026-10-17T06:38:31Z"
}
```

`engine_version` is the workspace package version. The current value is `0.1.0`.

`input.file` is the name inside the folder. `input.sha256` and `input.bytes` describe the input bytes. For `examples/divider/netlist.json` the length is 902.

`seed` is the netlist seed. When parsing fails, `seed` is `null`.

`created_at` is UTC, RFC 3339, at a whole second. It is the last field.

## netlist.json

This file is a byte-for-byte copy of the input.

## diagnostics.json

A JSON array of diagnostics, in the order defined in [diagnostics.md](diagnostics.md). An empty list is `[]`.

## observations.json

A successful operating point is one object under `op`. That object maps each ref to a number. Keys follow the ref order in [netlist.md](netlist.md).

The divider in [netlist.md](netlist.md) writes:

```json
{
  "op": {
    "#net.gnd": 0.0,
    "#net.vin": 5.0,
    "#net.vout": 3.333333331111111,
    "#r1.i": 0.001666666668888889,
    "#r1.v": 1.666666668888889,
    "#r1.p": 0.0027777777851851855,
    "#r2.i": 0.0016666666655555556,
    "#r2.v": 3.333333331111111,
    "#r2.p": 0.005555555548148148,
    "#v1.i": -0.001666666673888888,
    "#v1.v": 5.0,
    "#v1.p": -0.00833333336944444
  }
}
```

Numbers are serde_json's shortest round trip of an f64. `-0.0` is written as `0.0`.

The solve stamps 1e-12 S from every node to ground. That term puts `#net.vout` about 2e-9 V under 10/3 V. The test bound is 1e-8 relative. The three part powers sum to 0 within 1e-9 W. The difference is the power in that conductance.

A rejected netlist, and a solve that reports an error, write `{}`.

## Determinism

One binary on one platform writes the same bytes for the same input bytes and the same `created_at`.

Reordering `parts` and `nets` in the JSON keeps `observations.json` and `diagnostics.json` identical. The copied `netlist.json` still follows the input, so that file changes when the input bytes change.

Two CLI runs of one file differ only in `created_at`.

## Replacing a directory

`sim run` creates `<dir>`. It replaces an empty directory. It replaces a previous run. A previous run is a non-empty directory whose `manifest.json` has exactly the fields in the example above. `run_format` is 0. `netlist_format` is 0. `input.file` is `netlist.json`. `input.sha256` is 64 lowercase hex characters. `created_at` is `YYYY-MM-DDTHH:MM:SSZ`. `seed` is present, as an integer or `null`.

It leaves a non-directory path alone and returns `io_error`. It leaves any other non-empty directory alone and returns `io_error`. A `manifest.json` that only has `{"app":"web"}` is not a previous run.

A crash can leave `<dir>.partial.<digits>` or `<dir>.replacing.<digits>` next to the destination. When `<dir>` is missing, the write moves the newest `<dir>.replacing.<digits>` directory, or symlink to a directory, back to `<dir>` before it checks whether `<dir>` may be replaced. A write that leaves `<dir>` in place also leaves those siblings in place. A write that replaces `<dir>` removes each sibling. A directory is removed as a directory. A file or a symlink is removed as a file. Removal never follows a symlink, so the directory it points to keeps its contents. The rename of the new folder onto `<dir>` commits the run. A failure while deleting the backup after that rename does not fail the run.

`<dir>.partial.<digits>` and `<dir>.replacing.<digits>` belong to the engine. The engine may remove them. Concurrent `sim run` processes that share one `--out` are unsupported.
