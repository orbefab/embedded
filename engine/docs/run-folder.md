# Run folder

`sim run <netlist.json> --out <dir>` writes four files. A rejected netlist writes the same four files. A transient run adds `traces/`, and `traces/logic.vcd` when a chip in the netlist has logic signals. An NE555 has them, and so does an ATmega328P. It adds `traces/serial.csv` when a chip has a USART, as an ATmega328P does.

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
    envelope.f64
    logic.vcd
    serial.csv
```

Each JSON file is pretty JSON with a two-space indent and a trailing newline. `manifest.json` uses the field order in the example below. Keys inside a diagnostic `data` object are alphabetical. Observation keys follow ref order.

## manifest.json

```json
{
  "run_format": 0,
  "netlist_format": 0,
  "engine_version": "0.12.0",
  "input": {
    "file": "netlist.json",
    "sha256": "<lowercase hex sha256 of the input bytes>",
    "bytes": 904
  },
  "seed": 0,
  "created_at": "2026-10-17T06:38:31Z"
}
```

`run_format` versions this folder, and `engine_version` is the workspace package version. The current values are 0 and `0.12.0`. [versions.md](versions.md) says which changes move them.

`netlist_format` is the input's `version`, when it reads as an unsigned integer. That holds when the engine rejected the input, so `version: 1` records 1. It is `null` when no such integer could be read.

`input.file` is the name inside the folder. `input.sha256` and `input.bytes` describe the input bytes. For `examples/divider/netlist.json` the length is 904.

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

The solve stamps 1e-12 S from every net to ground. That term puts `#net.vout` about 2e-9 V under 10/3 V. The test bound is 1e-8 relative. The three part powers sum to 0 within 1e-9 W. The difference is the power in that conductance.

A transient writes `end_ps`, then `op`, then `switches`. `end_ps` is the last time the run solved, in picoseconds. It equals `stop_ps` exactly when the run reported no error. `op` is the operating point at t = 0, the same object an `op` run writes. `switches` maps each switch ref to every contact edge before `end_ps`, nominal and bounce, in time order. A switch with no edges has an empty list. `examples/rc-switch` writes:

```json
"switches": {
  "#sw1": [
    { "t_ps": 1000000000, "state": "closed" },
    { "t_ps": 31000000000, "state": "open" }
  ]
}
```

A netlist whose chips have logic signals adds `logic` after `switches`. It maps each logic ref to a summary of that signal up to `end_ps`. Every chip signal is listed, inputs included, so an agent can check whether and when each comparator flipped. An NE555 input's level is what its comparator reads, and OUT's is the level the latch drives. An ATmega328P's signals are its watched pins, each the level that pin reads, and `VCC` while brown-out is fused on, which is low while the supply holds the chip in brown-out reset ([netlist.md](netlist.md#refs)). A pin the firmware drives reads the level its drive makes, from the picosecond of the drive's stop. An input reads its level from the picosecond its threshold is crossed, which is where the core takes it. A drive change or a source jump at exactly `stop_ps` is not in the run, since no restart runs there ([engine.md](engine.md#chip-events)).

| Key | Meaning |
| --- | --- |
| `initial` | The level at t = 0. |
| `final` | The level after the last change. |
| `count` | How many changes the signal made after t = 0. |
| `first_ps` | The time of the first change, or `null` when `count` is 0. |
| `last_ps` | The time of the last change, or `null` when `count` is 0. |

Levels are `low` and `high`, so the first change is to the level opposite `initial`, and the last is to `final`. Every change is in `traces/logic.vcd`, which the summary is computed from. `examples/ne555-led` writes, abridged:

```json
"logic": {
  "#u1.logic.out": {
    "initial": "high",
    "final": "high",
    "count": 10,
    "first_ps": 626234627965,
    "last_ps": 3842809045627
  },
  "#u1.logic.reset": {
    "initial": "high",
    "final": "high",
    "count": 0,
    "first_ps": null,
    "last_ps": null
  }
}
```

The ATmega328P example in [netlist.md](netlist.md#atmega328p) toggles PB5 at cycles 4, 104, 204 and 304 of its 16 MHz crystal, 62,500 ps each, and writes, abridged:

```json
"logic": {
  "#u1.logic.PB5": {
    "initial": "low",
    "final": "low",
    "count": 4,
    "first_ps": 250000,
    "last_ps": 19000000
  },
  "#u1.logic.PC6": {
    "initial": "high",
    "final": "high",
    "count": 0,
    "first_ps": null,
    "last_ps": null
  }
}
```

The engine writes no period, duty or frequency. Which edges count, and whether the first cycle does, are the scenario's choice. The mean period over the falling edges of OUT, read from the VCD, is:

```python
def edges(vcd, ref):
    """[(t_ps, "low" | "high")] after t = 0 for one logic ref."""
    ids, out, at, scope = {}, [], 0, None
    for line in open(vcd).read().split("\n"):
        w = line.split()
        if w[:2] == ["$scope", "module"]: scope = w[2]
        elif w[:1] == ["$var"]: ids[w[3]] = f"#{scope}.logic.{w[4]}"
        elif line.startswith("#"): at = int(line[1:])
        elif line[:1] in ("0", "1") and ids.get(line[1:]) == ref and at > 0:
            out.append((at, "high" if line[0] == "1" else "low"))
    return out

falls = [t for t, level in edges("run/traces/logic.vcd", "#u1.logic.out") if level == "low"]
period_s = (falls[-1] - falls[0]) / (len(falls) - 1) * 1e-12
```

`observations.json` holds what the run reached. A rejected netlist, an operating point that failed, and a transient that failed at t = 0 write `{}`. A transient that fails after t = 0 writes `end_ps`, `op`, `switches` and `logic` up to the time it reached, the same keys a run that reaches `stop_ps` writes.

## traces

A transient writes a row at t = 0, at every multiple of `grid_ps` before `stop_ps`, and at `stop_ps`. That is `ceil(stop_ps / grid_ps) + 1` rows, fixed by the netlist before the run starts. A row is a probe: one solve from the last accepted step to the row time, which the run never accepts ([engine.md](engine.md#rows)). A row at a time a step lands on is that step. Each row is a solved point. The one exception is a row whose probe fails twice. It is the straight line between the accepted steps around it, and `trace.row_interpolated` names it. The grid never changes the accepted steps, the core stops, `logic.vcd`, `serial.csv`, `observations.json`, or the `solver.*` and `rating.*` diagnostics. Only two things depend on it. A grid past the row cap is refused with `trace.too_large` before the run solves. A row whose probe fails twice adds a `trace.row_interpolated` warning. The steps' tolerance is 1e-5 of each capacitor's swing plus 1 nV, so a small swing is held to its own size ([engine.md](engine.md#accuracy)).

A row is a point sample, and a waveform that repeats with the grid can alias the rows. The envelope is the between-rows read. Each column's minimum and maximum cover the two rows and every accepted step between them. A 0 to 3.3 V pulse at 1 MHz drives 100 ohm into 100 pF, a 10 ns time constant. On a 1 us grid every row falls on a rise and holds the level before it, so over 2 ms `#net.out0` spans 6.2e-9 V. Its envelope reads 0 to 3.3 V on every interval after the first. On a 100 ns grid the rows themselves read 0 to 3.3 V. The rating diagnostics and `logic.vcd` are read at every accepted step and never at a row's probe. `observations.switches` lists every edge at its picosecond. Those are the step-accurate views.

To read a waveform between rows as piecewise linear, set `grid_ps` under a tenth of the fastest time constant you read there. A chord across `h` is off by about `h^2/8` times the waveform's second derivative. For an exponential at a tenth of its time constant that is 1.25e-3 of its swing, and the chord's largest miss is 1.19e-3.

A row at the time of a contact edge, a source jump or a chip event holds the state before it. Later rows show the change. An edge, jump or event at `stop_ps` leaves the last row on the old state. Edges, jumps and chip events are landings for the steps, not rows. A jump at a row time appears in the envelope of the interval that starts at that row. The row holds the old state, and the accepted steps after it hold the new one, so the interval's range spans the jump. The restart solve after an edge is not a sample. The first accepted step after it is. That step is at most 1 ns after a switch edge, a source jump, or a chip's change that moves a capacitor. After a chip's drive change that moves none, it is the step the segment wanted, up to `max_step_ps` later, as [engine.md](engine.md#transient-steps) says. On the committed examples the restart sits at most 3e-13 V outside the step at the edge and the first step after it. On the aliasing case the restart sits about 4 mV outside.

`index.json` for `examples/rc-switch`, with the columns abridged:

```json
{
  "format": 0,
  "rows": 4051,
  "grid_ps": 20000000,
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
  },
  "envelope": {
    "file": "envelope.f64",
    "type": "f64le",
    "order": "column_major",
    "values": [
      "min",
      "max"
    ]
  }
}
```

- `format` is 0. A change that bumps it also bumps `run_format`.
- `grid_ps` and `stop_ps` are the netlist's.
- `time.u64` holds `rows` unsigned 64-bit little-endian integers, picoseconds from t = 0, strictly increasing. Row `k` is at `k * grid_ps`, and the last is at `stop_ps`. After an error the last row is at `end_ps`, which need not be on the grid.
- `analog.f64` holds `rows` IEEE 754 binary64 little-endian values per column, column after column. Column `j` occupies bytes `j * rows * 8` up to `(j + 1) * rows * 8`.
- `envelope.f64` holds one interval per pair of consecutive rows, `rows - 1` of them, as IEEE 754 binary64 little-endian values per column. The intervals' minima come first, then their maxima. Column `j` occupies bytes `j * (rows - 1) * 16` up to `(j + 1) * (rows - 1) * 16`. The first half of that span is the minima and the second half is the maxima. Interval `k` lies between rows `k` and `k + 1`. `time.u64` gives both boundaries. A trace with one row writes an empty file and still has the `envelope` key.
- An interval's minimum and maximum cover its two rows and every accepted step strictly between them. A row's probe is one of those rows, so it is a sample. A located chip event is an accepted step, so it is a sample. A restart solve is not, and neither is a refused step or a probe locating a chip event. The envelope has no column list. Its columns are `analog.columns`, in that order.
- `columns` are a subset of the keys of `observations.op`, in ref order. Read them from the index. Today every key of `op` is a column. A net or a `.v` is `V`, a `.i`, `.ib` or `.ic` is `A`, and a `.p` is `W`. Row 0 holds exactly the numbers in `op`.
- Every value of `analog.f64` and `envelope.f64` is finite. `-0.0` is written as `0.0`.
- There is no header and no compression.
- A chip adds no column. Its pins are nets, which have their columns, and its internal ladder node is not observed.

A netlist whose chips have logic signals adds a `logic` key after `envelope`:

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

`logic.vcd` holds every change of every chip signal, in Value Change Dump text. `observations.logic` is its summary.

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
#231119312840
1$
#626234627965
0!
1#
#626234627966
0#
```

The same example's `logic.vcd`, abridged to PB5 and the `$var` lines around it:

```text
$timescale 1 ps $end
$scope module u1 $end
$var wire 1 % PB4 $end
$var wire 1 & PB5 $end
$var wire 1 ' PC0 $end
$upscope $end
$enddefinitions $end
#0
$dumpvars
0&
$end
#250000
1&
#6500000
0&
#12750000
1&
#19000000
0&
```

- One `$scope module` per chip, in part order. Each signal's `$var` name is its bare name, in name order. An ATmega328P's run `PB0` to `PD7`, then `VCC` while brown-out is fused on.
- Identifiers are base 94 over the printable characters from `!`, by signal index in ref order, least significant first.
- `#0` with `$dumpvars` holds the levels at t = 0. Then one `#<ps>` line per time with a change, its changes in signal order.
- Times are integer picoseconds. There is no `$date` and no `$version`, so the file has no clock read and no float.
- After an error the file keeps the edges up to `end_ps`, like the rest of `traces/`.

A netlist with a chip that has a USART adds `serial.csv`. An ATmega328P has one, `USART0`. The file holds every serial frame a USART finished, one row per byte, as ASCII with `\n` line ends. `index.json` has no key for it, because its header names its columns. `examples/atmega-analog-serial` prints `512\r\n` three times, and its file starts:

```text
t_ps,part,usart,byte
254062500,u1,USART0,53
1294062500,u1,USART0,49
2334062500,u1,USART0,50
3374062500,u1,USART0,13
4414062500,u1,USART0,10
6598062500,u1,USART0,53
```

- `t_ps` is the start-bit time in integer picoseconds. It is the first picosecond at or after the oscillator cycle the start bit began, the time the core drove `TXD` low. When `TXD` read high before the start bit and nothing else loads it, that is the fall in `logic.vcd`. [avr.md](avr.md#sent-bytes) says when there is no fall at that time.
- `part` is the chip's part name and `usart` the USART's datasheet name. Neither needs quoting.
- `byte` is decimal, 0 to 255.
- Rows are in the order stop bits ended, then in part order, then in the chip's USART order. On one USART that is also the order bytes started. With two USARTs at different bauds, `t_ps` can step back between rows of different USARTs.
- A frame still on the wire at `stop_ps` is not in the file. After an error the file keeps the bytes whose stop bit ended by `end_ps`, like the rest of `traces/`.
- A chip with a USART writes the file even when it sent nothing, as the header alone. So a reader can tell a circuit with no USART from a USART that sent nothing.

Python reads the bytes each USART sent like this. The comment is what it prints for the whole file of `examples/atmega-analog-serial`. On a file that is the header alone it prints `{}`, and with no file `open` raises `FileNotFoundError`.

```python
import csv, collections
text = collections.defaultdict(bytearray)
for r in csv.DictReader(open("run/traces/serial.csv")): text[r["part"], r["usart"]].append(int(r["byte"]))
print({key: bytes(data) for key, data in text.items()})  # {('u1', 'USART0'): b'512\r\n512\r\n512\r\n'}
```

`end_ps` is the last row's time, and the same number as `observations.end_ps`. It is less than `stop_ps` only when the run reported an error. The grid rows before the error stay, and the last accepted step is the last row. A run that fails at t = 0 writes no `traces/`.

A transient whose grid gives more than `min(1000000, 67108864 / columns)` rows is refused before it solves, with one `trace.too_large`. 67108864 is 2^26 f64 values, 512 MiB. `columns` is the width of `analog.f64`. The cap counts `analog.f64` and not the envelope, which stores at most twice as many values. Writing the folder keeps a second copy of the trace bytes. A run at this cap peaks near 1.02 GiB without the envelope and near 3.02 GiB with it. The refused run writes `{}` to `observations.json` and no `traces/`. The row count depends only on `stop_ps` and `grid_ps`, so a run that is not refused never runs out of rows. Edges, jumps and chip events cost steps, not rows. Those steps can end the run at [`solver.step_limit`](diagnostics.md#solverstep_limit). `examples/rc-switch` writes 4051 rows, `examples/button-led` 2001, `examples/v-pulse-rc` 1401, `examples/current-pulse` 801, `examples/ne555-led` 401, `examples/atmega-blink` 401, `examples/atmega-button` 101, `examples/atmega-analog-serial` 951, `examples/atmega-fade` 15701, and `examples/atmega-serial-echo` 121.

Python with numpy reads one column like this:

```python
import json, numpy as np
ix = json.load(open("runs/rc/traces/index.json"))
t_ps = np.fromfile("runs/rc/traces/time.u64", "<u8")
refs = [c["ref"] for c in ix["analog"]["columns"]]
vout = np.fromfile("runs/rc/traces/analog.f64", "<f8").reshape(len(refs), ix["rows"])[refs.index("#net.vout")]
env = np.fromfile("runs/rc/traces/envelope.f64", "<f8").reshape(len(refs), 2, ix["rows"] - 1)
lo, hi = env[refs.index("#net.vout")]  # interval k spans t_ps[k] to t_ps[k + 1]
```

## Determinism

One binary on one platform writes the same bytes for the same input bytes and the same `created_at`.

Reordering `parts` and `nets` in the JSON keeps `observations.json` and `diagnostics.json` identical. The copied `netlist.json` still follows the input, so that file changes when the input bytes change.

Two CLI runs of one file differ only in `created_at`. That includes every byte of `traces/`, `logic.vcd` too. Trace values are the solver's f64 bits, written without a float printer.

## Replacing a directory

`sim run` creates `<dir>`. It replaces an empty directory. It replaces a previous run, from any version of the engine. A previous run is a non-empty directory whose `manifest.json` is an object with three keys of this shape. `run_format` is an unsigned integer, of any value. `input` is an object whose `file` is `netlist.json`. `created_at` is `YYYY-MM-DDTHH:MM:SSZ`. Every other key is ignored.

It leaves a non-directory path alone and returns `io_error`. It leaves any other non-empty directory alone and returns `io_error`. A `manifest.json` that only has `{"app":"web"}` is not a previous run.

A crash can leave `<dir>.partial.<digits>` or `<dir>.replacing.<digits>` next to the destination. When `<dir>` is missing, the write moves the newest `<dir>.replacing.<digits>` directory, or symlink to a directory, back to `<dir>` before it checks whether `<dir>` may be replaced. A write that leaves `<dir>` in place also leaves those siblings in place. A write that replaces `<dir>` removes each sibling. A directory is removed as a directory. A file or a symlink is removed as a file. Removal never follows a symlink, so the directory it points to keeps its contents. The rename of the new folder onto `<dir>` commits the run. A failure while deleting the backup after that rename does not fail the run.

Whether a directory is a previous run depends on its manifest alone, so a transient run replaces an operating point run and the other way round. A replaced run's `traces/` goes with it.

`<dir>.partial.<digits>` and `<dir>.replacing.<digits>` belong to the engine. The engine may remove them. Concurrent `sim run` processes that share one `--out` are unsupported.
