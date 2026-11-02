# Run folder

`sim run <netlist.json> --out <dir>` writes four files. A rejected netlist writes the same four files. A transient run adds `traces/`, and `traces/logic.vcd` when a chip in the netlist has logic signals. An NE555 has them, and so do an ATmega328P and an ESP32-S3. It adds `traces/serial.csv` when a chip has a USART, as an ATmega328P does, or a console, as an ESP32-S3 does, `traces/i2c.csv` when a chip has a TWI, as an ATmega328P does, `traces/link.csv` when a chip can send records, as an SPI replay and an ESP32-S3 do, `traces/display.csv` and one `traces/display-<part>.rgb565` for each display part, an `st77916`, and `traces/leds.csv` when a `ws2812` is in the netlist.

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
    i2c.csv
    link.csv
    display.csv
    display-<part>.rgb565
    leds.csv
```

Each JSON file is pretty JSON with a two-space indent and a trailing newline. `manifest.json` uses the field order in the example below. Keys inside a diagnostic `data` object are alphabetical. Observation keys follow ref order.

## manifest.json

```json
{
  "run_format": 0,
  "netlist_format": 0,
  "engine_version": "0.42.0",
  "input": {
    "file": "netlist.json",
    "sha256": "<lowercase hex sha256 of the input bytes>",
    "bytes": 904
  },
  "seed": 0,
  "created_at": "2026-10-17T06:38:31Z"
}
```

`run_format` versions this folder, and `engine_version` is the workspace package version. The current values are 0 and `0.42.0`. [versions.md](versions.md) says which changes move them.

`netlist_format` is the input's `version`, when it reads as an unsigned integer. That holds when the engine rejected the input, so `version: 1` records 1. It is `null` when no such integer could be read.

`input.file` is the name inside the folder. `input.sha256` and `input.bytes` describe the input bytes. For `examples/divider/netlist.json` the length is 904.

`blobs` follows `input` when the netlist names a [blob](netlist.md#blobs), and is absent otherwise, so a folder whose netlist names none has no such key. It lists each blob in part order, and in each part's field order, with an `esp32s3` part's `rom` before its firmware. An entry is `{ "role", "sha256", "bytes" }`, and a flash region's adds `offset` after `bytes`. `role` is `elf` for an `esp32s3` part's `firmware.elf`, `rom` for its `rom`, and `flash` for each region of its `firmware.flash`, in the netlist's order. `examples/esp32s3-blink` writes:

```json
  "blobs": [
    {
      "role": "elf",
      "sha256": "36f33508452c63a7527edff29db0da3e534714c5737c660a441a1c056effe3aa",
      "bytes": 3800
    }
  ],
```

When parsing fails, `blobs` is absent. The folder copies no blob. A rerun from the folder alone fails with `netlist.blob`, so it names the blob folder, as in `sim run <run>/netlist.json --out <new> --blobs examples/esp32s3-blink/blobs`. A rerun from `examples/esp32s3-blink/netlist.json` finds the `blobs` folder beside it. No example commits the ROM, so a rerun of `examples/esp32s3-hello` also passes the folder `scripts/blobs.sh` prints, as in `sim run examples/esp32s3-hello/netlist.json --out <new> --blobs "$(scripts/blobs.sh)"`.

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

A netlist with a display part adds `displays` after `logic`. It maps each display part's name to its state at `end_ps`. `kind` is the part's kind, `width` and `height` are its pixels, and `format` is how its frame memory file holds a pixel. `flags` holds `display_on`, `inverted`, `reset`, which is whether its reset pin reads low, and `sleep`. `madctl` and `colmod` are those registers as integers. `registers` is how many registers hold a stored last write, at most 256. `brightness` is the last duty its backlight pin took, as `duty` of `steps` at `hz`, or `null` before the first. `image` is its frame memory file and `sha256` that file's SHA-256 in lowercase hex. `examples/st77916-replay` writes:

```json
"displays": {
  "lcd1": {
    "kind": "st77916",
    "width": 360,
    "height": 360,
    "format": "rgb565",
    "flags": {
      "display_on": true,
      "inverted": false,
      "reset": false,
      "sleep": false
    },
    "madctl": 0,
    "colmod": 85,
    "registers": 4,
    "brightness": null,
    "image": "traces/display-lcd1.rgb565",
    "sha256": "371c18ddc4bcc728db49b5919fea8cfa7d6f3a0469fe75b5fc2a8a9fe34e15cb"
  }
}
```

A netlist with a `ws2812` adds `leds` after `displays`. It maps each chain's name to its colours at `end_ps`. `kind` is `ws2812`, `count` is its LED count, `order` is `grb`, and `pixels` is one `[green, red, blue]` per LED, LED 0 first, each channel an integer from 0 to 255. A chain that never latched is still present, with every pixel `[0, 0, 0]`. `examples/ws2812-replay` writes one chain of 13:

```json
"leds": {
  "ring": {
    "kind": "ws2812",
    "count": 13,
    "order": "grb",
    "pixels": [[0, 16, 32], [1, 17, 33], [2, 18, 34], [3, 19, 35], [4, 20, 36], [5, 21, 37], [6, 22, 38], [7, 23, 39], [8, 24, 40], [9, 25, 41], [10, 26, 42], [11, 27, 43], [12, 28, 44]]
  }
}
```

`observations.json` holds what the run reached. A rejected netlist, an operating point that failed, and a transient that failed at t = 0 write `{}`. A transient that fails after t = 0 writes `end_ps`, `op`, `switches`, `logic`, `displays` and `leds` up to the time it reached, the same keys a run that reaches `stop_ps` writes.

## traces

A transient writes a row at t = 0, at every multiple of `grid_ps` before `stop_ps`, and at `stop_ps`. That is `ceil(stop_ps / grid_ps) + 1` rows, fixed by the netlist before the run starts. A row is a probe: one solve from the last accepted step to the row time, which the run never accepts ([engine.md](engine.md#rows)). A row at a time a step lands on is that step. Each row is a solved point. The one exception is a row whose probe fails twice. It is the straight line between the accepted steps around it, and `trace.row_interpolated` names it. The grid never changes the accepted steps, the core stops, `logic.vcd`, `serial.csv`, `observations.json`, or the `solver.*` and `rating.*` diagnostics. Only two things depend on it. A grid past the row cap is refused with `trace.too_large` before the run solves. A row whose probe fails twice adds a `trace.row_interpolated` warning. The steps' tolerance is 1e-5 of each capacitor's swing plus 1 nV, and 1e-5 of each inductor's current swing plus 1e-12 A. The voltage an inductor's current error implies is held to 1e-5 of that voltage's swing plus 1 nV. So a small swing is held to its own size ([engine.md](engine.md#accuracy)).

A row is a point sample, and a waveform that repeats with the grid can alias the rows. The envelope is the between-rows read. Each column's minimum and maximum cover the two rows and every accepted step between them. A 0 to 3.3 V pulse at 1 MHz drives 100 ohm into 100 pF, a 10 ns time constant. On a 1 us grid every row falls on a rise and holds the level before it, so over 2 ms `#net.out0` spans 6.2e-9 V. Its envelope reads 0 to 3.3 V on every interval after the first. On a 100 ns grid the rows themselves read 0 to 3.3 V. The rating diagnostics and `logic.vcd` are read at every accepted step and never at a row's probe. `observations.switches` lists every edge at its picosecond. Those are the step-accurate views.

To read a waveform between rows as piecewise linear, set `grid_ps` under a tenth of the fastest time constant you read there. A chord across `h` is off by about `h^2/8` times the waveform's second derivative. For an exponential at a tenth of its time constant that is 1.25e-3 of its swing, and the chord's largest miss is 1.19e-3.

A row at the time of a contact edge, a source jump or a chip event holds the state before it. Later rows show the change. An edge, jump or event at `stop_ps` leaves the last row on the old state. Edges, jumps and chip events are landings for the steps, not rows. A jump at a row time appears in the envelope of the interval that starts at that row. The row holds the old state, and the accepted steps after it hold the new one, so the interval's range spans the jump. The restart solve after an edge is not a sample. The first accepted step after it is. That step is at most 1 ns after a switch edge, a source jump, or a chip's change whose jump exceeds Newton's tolerance on a capacitor or an inductor. After a chip's drive change whose jump is within that tolerance, it is the step the segment wanted, up to `max_step_ps` later, as [engine.md](engine.md#transient-steps) says. On the committed examples the restart sits at most 3e-13 V outside the step at the edge and the first step after it. On the aliasing case the restart sits about 4 mV outside.

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
- `columns` are a subset of the keys of `observations.op`, in ref order. Read them from the index. Today every key of `op` is a column. A net or a `.v` is `V`, a `.i`, `.ib`, `.ic`, `.id`, `.iin` or `.iout` is `A`, and a `.p` is `W`. Row 0 holds exactly the numbers in `op`.
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

A netlist with a chip that has a USART adds `serial.csv`. An ATmega328P has one, `USART0`. An ESP32-S3's USB Serial/JTAG console counts as one, `USB_SERIAL_JTAG`. The file holds every serial frame a USART finished, one row per byte, as ASCII with `\n` line ends. `index.json` has no key for it, because its header names its columns. `examples/atmega-analog-serial` prints `512\r\n` three times, and its file starts:

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
- On `USB_SERIAL_JTAG`, `t_ps` is the time the byte's packet went, and every byte of one packet has that time, as [esp32s3.md](esp32s3.md#what-it-models) says.
- `part` is the chip's part name and `usart` the USART's datasheet name. Neither needs quoting.
- `byte` is decimal, 0 to 255.
- Rows are in the order stop bits ended, then by part name, then by the USART's index in `usarts`, then in the order its chip sent them. Parse sorts parts by name, so that index order is the part order a parsed netlist has. On one USART that is also the order bytes started. With two USARTs at different bauds, `t_ps` can step back between rows of different USARTs.
- A frame still on the wire at `stop_ps` is not in the file. After an error the file keeps the bytes whose stop bit ended by `end_ps`, like the rest of `traces/`.
- A chip with a USART writes the file even when it sent nothing, as the header alone. So a reader can tell a circuit with no USART from a USART that sent nothing.

Python reads the bytes each USART sent like this. The comment is what it prints for the whole file of `examples/atmega-analog-serial`. On a file that is the header alone it prints `{}`, and with no file `open` raises `FileNotFoundError`.

```python
import csv, collections
text = collections.defaultdict(bytearray)
for r in csv.DictReader(open("run/traces/serial.csv")): text[r["part"], r["usart"]].append(int(r["byte"]))
print({key: bytes(data) for key, data in text.items()})  # {('u1', 'USART0'): b'512\r\n512\r\n512\r\n'}
```

A netlist with a chip that has a TWI adds `i2c.csv`. An ATmega328P has one. The file holds every phase the TWI master saw on the bus, one row per phase, as ASCII with `\n` line ends. `index.json` has no key for it, because its header names its columns. `examples/atmega-i2c-scan` probes addresses 1 to 126 with nothing on the bus, and its file starts:

```text
t_ps,part,phase,address,read,byte,ack
218062500,u1,start,,,,
232375000,u1,address,1,0,,0
322000000,u1,stop,,,,
349375000,u1,start,,,,
363687500,u1,address,2,0,,0
453312500,u1,stop,,,,
```

- `phase` is `start`, `repeated_start`, `address`, `data`, `stop`, `arbitration_lost` or `bus_error`.
- `t_ps` is the phase's first bus change, the first SCL or SDA edge the TWI read after the cycle the phase before it ended, while it owned the bus or had a phase under way. It is the first picosecond at or after that oscillator cycle. A START and a repeated START end at their SCL fall, an address or data byte at its ninth SCL fall, a STOP when SDA reads high, an arbitration loss at the SCL rise that read it, and a bus error at the SDA edge.
- `part` is the chip's part name.
- On an address row `address` is the 7-bit address, and on a data row `byte` is the byte, both decimal. `read` is 1 for a read and 0 for a write, and `ack` is 1 for an acknowledge and 0 for none, on both. On a write the target acknowledges, and on a read the master does. Every other cell is empty.
- Rows of one delivery are in the order phases ended, then in part order. A phase that ends on an input can end within one CPU cycle before a delivery already made, and that row is then written after a row that ended later.
- A phase still on the bus at `stop_ps` is not in the file. After an error the file keeps the phases that ended by `end_ps`.
- A chip with a TWI writes the file even when the TWI never ran, as the header alone.

`scripts/i2c_decode.py` decodes the bus from `logic.vcd` apart from the engine and checks it against this file, as [reference.md](reference.md#i2c) says.

A netlist with a chip that can send records adds `link.csv`. An SPI replay, a pulse replay and an ESP32-S3 can. The file holds every record a chip sent, one row per record, as ASCII with `\n` line ends. `index.json` has no key for it, because its header names its columns. `examples/spi-replay` replays a display panel's start-up, and its file is:

```text
t_ps,start_ps,part,pin,kind,cs,phases,bytes,hash,lanes
10800000,10000000,rp1,CS,spi,released,8/1:02 24/1:001100,0,e3b0c44298fc1c14,1
21000000,20000000,rp1,CS,spi,released,8/1:02 24/1:003a00,1,a25513c7e0f6eaa8,1
31600000,30000000,rp1,CS,spi,released,8/1:02 24/1:002a00,4,b40711a88c703975,1
41600000,40000000,rp1,CS,spi,released,8/1:02 24/1:002b00,4,df3f619804a92fdb,1
50900000,50000000,rp1,CS,spi,held,8/1:32 24/1:002c00,2,240fbad97784f181,4
52100000,52000000,rp1,CS,spi,released,,2,98cebd64dc25ea9c,4
60800000,60000000,rp1,CS,spi,released,8/1:02 24/1:002900,0,e3b0c44298fc1c14,1
```

- `t_ps` is the record's end and `start_ps` is its start, both in integer picoseconds. `part` is the sender's part name. `pin` names the link on the sender. It is `CS` for a replay's `spi` record. For an ESP32-S3's it is the pin its SPI2 chip select is routed to, such as `GPIO12`, or `SPI2` when CS is routed to none of the part's pins.
- `kind` is `spi`, `pulses` or `duty`. At this version an `spi_replay` and an `esp32s3` write `spi` rows, a `pulses_replay` writes `pulses` rows, and an `esp32s3` writes `duty` rows from its LEDC.
- On a `spi` row, `cs` is `held` or `released`, the chip select at the record's end. `phases` lists each phase, space-separated, as `<bits>/<lanes>:<value>`, with the value in lowercase hex, one digit for each four bits or part of four. `bytes` is the data's length, `hash` is the first 8 bytes of the data's SHA-256, in lowercase hex, and `lanes` is how many lanes the data went on, 1, 2 or 4.
- On a `pulses` row, `phases` is `idle=0` or `idle=1`. The payload is each pulse as one level byte and its time as a little-endian `u64` of picoseconds, and `bytes` and `hash` are its length and hash.
- On a `duty` row, `phases` is `<duty>/<2^resolution>@<hz>`, and `bytes`, `hash` and `lanes` are empty.
- Every other cell is empty.
- Rows are in the order records ended, then by sender part name, then in the order each sender sent them.
- A record that ends after `stop_ps` is not in the file. After an error the file keeps the records that ended by `end_ps`.
- A chip that can send records writes the file even when it sent none, as the header alone.

A record is not a level, so no column of `analog.f64` and no signal of `logic.vcd` shows it. The replay's pins hold their idle levels in the trace for the whole run, and an ESP32-S3's SPI2 pads hold theirs while they are routed.

A netlist with a display part adds `display.csv`. The file holds every pixel write a display part closed, one row per write, as ASCII with `\n` line ends. `index.json` has no key for it. `examples/st77916-replay` closes seven writes, and its file is:

```text
t_ps,part,x0,y0,x1,y1,bytes,hash
217900000,lcd1,20,24,99,68,7200,ee5398912b7c7604
305900000,lcd1,200,40,239,79,3200,035803db8411b1ee
418800000,lcd1,60,150,123,181,4096,1b59da792997e1c8
568300000,lcd1,300,200,329,289,5400,8bb51a3a998d6750
576650000,lcd1,0,0,1,2,14,5e6168bb2c6231b4
584850000,lcd1,350,350,353,351,8,df7c398fe7d1b2b3
587450000,lcd1,350,350,353,351,8,83c2e68d92ed8469
```

- `t_ps` is the time the write closed, the end of the record that closed it, in integer picoseconds. `part` is the display part's name.
- `x0`, `y0`, `x1` and `y1` name the window, first and last column and row, both ends included, before the memory access order maps them. A `RAMWR` begins at that window's top-left. A `RAMWRC` row names the same window, not the column and row where the continuation started. A write that wraps past the window's end still names that window.
- `bytes` is how many bytes the write took, and `hash` is the first 8 bytes of their SHA-256, in lowercase hex.
- Rows are in the order writes closed, then by part name, then in the order each part closed them.
- A write still open at `stop_ps`, a write that took no bytes, and a write a reset dropped are not in the file. After an error the file keeps the writes that closed by `end_ps`.
- A display part writes the file even when it closed no write, as the header alone.

Each display part also adds `display-<part>.rgb565`, its frame memory at `end_ps`. An `st77916` file is 259200 bytes, 360 rows of 360 pixels, rows top to bottom and each row left to right. The file keeps each pixel's bytes in the order they arrived. Firmware that sends the high byte first, as `examples/st77916-replay` does and as `COLMOD` 0x55 on this panel expects, gives big-endian RGB565. A reader must not assume little-endian. A pixel no write reached is two zero bytes. [netlist.md](netlist.md#st77916) says how a write maps its pixels, and [What the glass shows](#what-the-glass-shows) how a reader turns the file into the pixels a person sees.

`scripts/render_st77916.py` plays a replay list into a panel apart from the engine and checks a run's `display.csv`, without `t_ps`, its frame memory file, and the `displays` entry's `registers`, `flags`, `madctl` and `colmod` against it, as [reference.md](reference.md#st77916) says.

A netlist with a `ws2812` adds `leds.csv`. The file holds every latch, one row per latch, as ASCII with `\n` line ends. `index.json` has no key for it. A latch is the chain after a low longer than the reset. `examples/ws2812-replay` latches once, and its file is:

```text
t_ps,part,count,grb
1669062501,ring,13,0010200111210212220313230414240515250616260717270818280919290a1a2a0b1b2b0c1c2c
```

- `t_ps` is the reset, in integer picoseconds. That instant can fall inside a pulse, before the record that carried the bits ends. `part` is the chain's name. `count` is its LED count.
- `grb` is green, red, then blue for LED 0 onward, six lowercase hex digits each, with no separators.
- Rows are in order of time, then by part name.
- A frame still open at `stop_ps` is not a row. After an error the file keeps the latches at or before `end_ps`.
- A `ws2812` writes the file even when it never latched, as the header alone.

`scripts/ws2812_decode.py` reads `DIN` in `traces/logic.vcd` apart from the engine and checks the rows and the `leds` entry, as [reference.md](reference.md#skips) says. A `pulses_replay` holds its pin at the idle level, so the bits of `examples/ws2812-replay` are in the record and not in that VCD.

`end_ps` is the last row's time, and the same number as `observations.end_ps`. It is less than `stop_ps` only when the run reported an error. The grid rows before the error stay, and the last accepted step is the last row. A run that fails at t = 0 writes no `traces/`.

A transient whose grid gives more than `min(1000000, 67108864 / columns)` rows is refused before it solves, with one `trace.too_large`. 67108864 is 2^26 f64 values, 512 MiB. `columns` is the width of `analog.f64`. The cap counts `analog.f64` and not the envelope, which stores at most twice as many values. Writing the folder keeps a second copy of the trace bytes. A run at this cap peaks near 1.02 GiB without the envelope and near 3.02 GiB with it. The refused run writes `{}` to `observations.json` and no `traces/`. The row count depends only on `stop_ps` and `grid_ps`, so a run that is not refused never runs out of rows. Edges, jumps and chip events cost steps, not rows. Those steps can end the run at [`solver.step_limit`](diagnostics.md#solverstep_limit). `examples/rc-switch` writes 4051 rows, `examples/button-led` 2001, `examples/v-pulse-rc` 1401, `examples/current-pulse` 801, `examples/ne555-led` 401, `examples/atmega-blink` 401, `examples/atmega-board-blink` 401, `examples/atmega-board-button` 21, `examples/atmega-board-knob` 111, `examples/atmega-button` 101, `examples/atmega-analog-serial` 951, `examples/atmega-fade` 15701, `examples/atmega-serial-echo` 121, `examples/atmega-i2c-scan` 201, `examples/npn-sweep` 521, `examples/pnp-sweep` 521, `examples/nmos-sweep` 521, `examples/npn-pulse` 1101, `examples/st77916-replay` 41, `examples/atmega-ws2812` 21, `examples/ws2812-replay` 5, and `examples/npn-blinker` 2001.

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

## What the glass shows

The frame memory file is what the panel holds, not what a person sees. The engine draws what the glass shows, and a viewer only paints those pixels. `sim view <dir> --out <out>` reads a finished run folder and writes `display-<part>.png` into `<out>` for each entry of `observations.json` `displays`. It writes nothing into the run folder unless `<out>` is that folder. `Display::pixels` on a session and `displayPixels` in the wasm package give the same pixels from the same function, as [engine.md](engine.md#sessions) and [wasm.md](wasm.md) say.

For an `st77916` the pixels are 360 rows of 360, rows top to bottom and each row left to right, each pixel red, green, blue and alpha, one byte each:

- A pixel whose centre lies outside the round glass is transparent, `[0, 0, 0, 0]`. [netlist.md](netlist.md#st77916) defines the circle. 101780 of the 129600 pixels are on the glass.
- When `display_on` is false, `sleep` is true, or `reset` is true, every pixel on the glass is opaque black, `[0, 0, 0, 255]`.
- Otherwise a pixel is its two frame memory bytes read as big-endian RGB565, high byte first. The frame memory holds only pixels written while `COLMOD`'s low three bits are 5, 16 bits a pixel, as 0x55 is. The panel drops a write in any other format, so `colmod` at the end does not change how a pixel reads. Each channel widens to 8 bits by bit replication: the channel's top bits repeat below it, so 5 bits `v` become `v << 3 | v >> 2` and 6 bits become `v << 2 | v >> 4`. Replication keeps 0 at 0 and takes full scale to 255, lands every value within one of `v * 255 / full scale`, and needs no table or division.
- When `inverted` is true the pixel is shown as written. When it is false each channel is inverted, `255 - c`. That is this panel's sense of `INVON`, a board observation [netlist.md](netlist.md#st77916) records.
- Alpha is 255 on the glass.

The glass does not dim. `brightness` stays a separate number in the `displays` entry, and a viewer applies it.

`examples/st77916-replay` ends with `inverted` false, so its unwritten pixels, all zero bytes, show white on the glass.

The PNG is 8-bit RGBA with no interlace. Every row has filter type 0, and the image data is one `IDAT` of stored deflate blocks, uncompressed. The same pixels always give the same bytes, so two `sim view` runs of one folder write identical files. An `st77916` PNG is 518863 bytes.

`sim view` prints one JSON document. A clean view is `ok`, exit 0, with `out` and `images`, the PNG path written for each part, by part name. A run folder with no display part creates no `<out>`, writes no file, and prints `"images": {}`. A folder it cannot read is `io_error`, exit 3, with `message` and `path`, and it writes no file. That covers a missing or malformed `observations.json`, a display kind it does not draw, an `image` other than `traces/display-<part>.rgb565`, and a frame memory file that is missing, the wrong size, or not the bytes its `sha256` names.

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
