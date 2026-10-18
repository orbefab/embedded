# Run folder

`sim run <netlist.json> --out <dir>` writes four files. A rejected netlist writes the same four files. A transient run adds `traces/`, and `traces/logic.vcd` when the netlist has a chip.

```text
<dir>/
  manifest.json
  netlist.json
  diagnostics.json
  observations.json
  traces/
    index.json
    time.u64
    analog.f64
    logic.vcd
```

Each JSON file is pretty JSON with a two-space indent and a trailing newline. `manifest.json` uses the field order in the example below. Keys inside a diagnostic `data` object are alphabetical. Observation keys follow ref order.

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

A transient adds `switches` after `op`. `op` is the operating point at t = 0, the same object an `op` run writes. `switches` maps each switch ref to every contact edge before `stop_ps`, nominal and bounce, in time order. A switch with no edges has an empty list. `examples/rc-switch` writes:

```json
"switches": {
  "#sw1": [
    { "t_ps": 1000000000, "state": "closed" },
    { "t_ps": 31000000000, "state": "open" }
  ]
}
```

A netlist with a chip adds `logic` after `switches`. It maps each logic ref to its level at t = 0 and every change after it, in time order. Every chip signal is listed, inputs included, so an agent can check when each comparator flipped. An input's level is what its comparator reads, and OUT's is the level the latch drives. `examples/ne555-led` writes, abridged:

```json
"logic": {
  "#u1.logic.out": {
    "initial": "high",
    "edges": [
      { "t_ps": 626237476311, "level": "low" },
      { "t_ps": 953460598901, "level": "high" }
    ]
  },
  "#u1.logic.reset": { "initial": "high", "edges": [] },
  "#u1.logic.thres": {
    "initial": "low",
    "edges": [
      { "t_ps": 626237476311, "level": "high" },
      { "t_ps": 626237476312, "level": "low" }
    ]
  },
  "#u1.logic.trig": {
    "initial": "low",
    "edges": [{ "t_ps": 231122065116, "level": "high" }]
  }
}
```

The engine writes no period, duty or frequency. Which edges count, and whether the first cycle does, are the scenario's choice. The mean period over the falling edges of OUT is:

```python
falls = [e["t_ps"] for e in obs["logic"]["#u1.logic.out"]["edges"] if e["level"] == "low"]
period_s = (falls[-1] - falls[0]) / (len(falls) - 1) * 1e-12
```

A rejected netlist, and a solve that reports an error, write `{}`.

## traces

A transient writes one row per accepted time step, plus row 0 at t = 0. The rows are not evenly spaced. Read the trace as piecewise linear between rows. Each row is within the error control's tolerance, which is relative to each capacitor's voltage, so a swing much smaller than its level carries the level's error ([reference.md](reference.md#the-comparison)). Between two rows `h` apart, a linear read can be off by about `h^2/8` times the waveform's second derivative. A smaller `max_step_ps` makes the trace denser. At a contact edge before `stop_ps` there is a row at the edge, which holds the old contact, and a row at the earlier of 1 ns later and `stop_ps`, which holds the new contact. An edge at `stop_ps` has one row, which holds the old contact, and there is no later row. A source jump follows the same rule, and its first row holds the old level. An edge on another switch inside that nanosecond does not remove either row. A fast waveform can add rows between them, where the error control shortens the step. At a chip event there is a row at the located picosecond, which holds the chips' old states. The new drive shows from the next row on.

`index.json` for `examples/rc-switch`, with the columns abridged:

```json
{
  "format": 0,
  "rows": 4093,
  "stop_ps": 81000000000,
  "end_ps": 81000000000,
  "time": {
    "file": "time.u64",
    "type": "u64le",
    "unit": "ps"
  },
  "analog": {
    "file": "analog.f64",
    "type": "f64le",
    "order": "column_major",
    "columns": [
      {
        "ref": "#net.gnd",
        "unit": "V"
      },
      {
        "ref": "#c1.i",
        "unit": "A"
      }
    ]
  }
}
```

- `time.u64` holds `rows` unsigned 64-bit little-endian integers, picoseconds from t = 0, strictly increasing. Row 0 is t = 0.
- `analog.f64` holds `rows` IEEE 754 binary64 little-endian values per column, column after column. Column `j` occupies bytes `j * rows * 8` up to `(j + 1) * rows * 8`.
- `columns` are the keys of `observations.op`, in the same order. A net or a `.v` is `V`, a `.i` is `A`, and a `.p` is `W`. Row 0 holds exactly the numbers in `op`.
- Every value is finite. `-0.0` is written as `0.0`.
- There is no header and no compression.
- A chip adds no column. Its pins are nets, which have their columns, and its internal ladder node is not observed.

A netlist with a chip adds a `logic` key after `analog`:

```json
"logic": {
  "file": "logic.vcd",
  "type": "vcd",
  "timescale": "1 ps",
  "signals": [
    { "ref": "#u1.logic.out", "scope": "u1", "var": "out", "id": "!" },
    { "ref": "#u1.logic.reset", "scope": "u1", "var": "reset", "id": "\"" },
    { "ref": "#u1.logic.thres", "scope": "u1", "var": "thres", "id": "#" },
    { "ref": "#u1.logic.trig", "scope": "u1", "var": "trig", "id": "$" }
  ]
}
```

`logic.vcd` holds the same levels as `observations.logic`, in Value Change Dump text:

```text
$timescale 1 ps $end
$scope module u1 $end
$var wire 1 ! out $end
$var wire 1 " reset $end
$var wire 1 # thres $end
$var wire 1 $ trig $end
$upscope $end
$enddefinitions $end
#0
$dumpvars
1!
1"
0#
0$
$end
#231122065116
1$
#626237476311
0!
1#
#626237476312
0#
```

- One `$scope module` per chip, in part order. Each signal's `$var` name is its bare name, in name order.
- Identifiers are base 94 over the printable characters from `!`, by signal index in ref order, least significant first.
- `#0` with `$dumpvars` holds the levels at t = 0. Then one `#<ps>` line per time with a change, its changes in signal order.
- Times are integer picoseconds. There is no `$date` and no `$version`, so the file has no clock read and no float.
- After an error the file keeps the edges up to the last row, like the rest of `traces/`.

`end_ps` is the last row's time. It is less than `stop_ps` only when the run reported an error. The rows before the error stay, while `observations.json` is `{}`. A run that fails at t = 0 writes no `traces/`. For `trace.too_large`, `end_ps` is still that last kept row, and the diagnostic's `t_ps` is the step that did not fit.

A run stops with `trace.too_large` at `min(1000000, 67108864 / columns)` rows. 67108864 is 2^26 f64 values, 512 MiB. `columns` is the width of `analog.f64`. A run needs at least `stop_ps / max_step_ps` rows, plus about `log2(max_step_ps / 1000)` for each edge or source jump, and more where the error control holds the step under `max_step_ps`. After each one the step doubles from the 1 ns restart step while the error allows. `examples/rc-switch` takes 4050 plus 43. `examples/current-pulse` takes 800 plus 109, about 18 for each of its 6 jumps. Each located chip event is a row. One that changes a chip's drive also restarts, and adds about `log2(max_step_ps / 1000)` rows like a switch edge. `examples/ne555-led` takes 400 plus 277, about 28 for each of its 10 output edges. A chip that switches often fills the trace with event rows whatever `max_step_ps` is.

Python with numpy reads one column like this:

```python
import json, numpy as np
ix = json.load(open("runs/rc/traces/index.json"))
t_ps = np.fromfile("runs/rc/traces/time.u64", "<u8")
refs = [c["ref"] for c in ix["analog"]["columns"]]
vout = np.fromfile("runs/rc/traces/analog.f64", "<f8").reshape(len(refs), ix["rows"])[refs.index("#net.vout")]
```

## Determinism

One binary on one platform writes the same bytes for the same input bytes and the same `created_at`.

Reordering `parts` and `nets` in the JSON keeps `observations.json` and `diagnostics.json` identical. The copied `netlist.json` still follows the input, so that file changes when the input bytes change.

Two CLI runs of one file differ only in `created_at`. That includes every byte of `traces/`, `logic.vcd` too. Trace values are the solver's f64 bits, written without a float printer.

## Replacing a directory

`sim run` creates `<dir>`. It replaces an empty directory. It replaces a previous run. A previous run is a non-empty directory whose `manifest.json` has exactly the fields in the example above. `run_format` is 0. `netlist_format` is 0. `input.file` is `netlist.json`. `input.sha256` is 64 lowercase hex characters. `created_at` is `YYYY-MM-DDTHH:MM:SSZ`. `seed` is present, as an integer or `null`.

It leaves a non-directory path alone and returns `io_error`. It leaves any other non-empty directory alone and returns `io_error`. A `manifest.json` that only has `{"app":"web"}` is not a previous run.

A crash can leave `<dir>.partial.<digits>` or `<dir>.replacing.<digits>` next to the destination. When `<dir>` is missing, the write moves the newest `<dir>.replacing.<digits>` directory, or symlink to a directory, back to `<dir>` before it checks whether `<dir>` may be replaced. A write that leaves `<dir>` in place also leaves those siblings in place. A write that replaces `<dir>` removes each sibling. A directory is removed as a directory. A file or a symlink is removed as a file. Removal never follows a symlink, so the directory it points to keeps its contents. The rename of the new folder onto `<dir>` commits the run. A failure while deleting the backup after that rename does not fail the run.

`run_format` stays 0 with `traces/`. Whether a directory is a previous run depends on its manifest alone, so a transient run replaces an operating point run and the other way round. A replaced run's `traces/` goes with it.

`<dir>.partial.<digits>` and `<dir>.replacing.<digits>` belong to the engine. The engine may remove them. Concurrent `sim run` processes that share one `--out` are unsupported.
