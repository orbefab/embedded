# Netlist v1

A netlist is fully resolved JSON. `version` is 1. [Version](#version) says which changes keep it. Every field is required except the top-level `world`, and an unknown field is an error. Nothing is filled in by default. A netlist without `world` declares no world quantity, as [World](#world) says.

The netlist is the engine's input. Data a part needs lives inside the document, with one exception. A large input, such as an `esp32s3` part's firmware or its mask ROM, is a blob, which the document names by its SHA-256 and length, as [Blobs](#blobs) says.

Values are SI numbers. `unit` must already be the field's unit. Integer values are the exception. A time in `ps` sits on the engine's picosecond timeline, and a count is in `1`.

## Document

`packages/engine/tests/circuits/divider/netlist.json` is a 5 V source across `vin` and `gnd`, 1000 ohm from `vin` to `vout`, and 2000 ohm from `vout` to `gnd`.

```json
{
  "version": 1,
  "seed": 0,
  "analysis": { "op": {} },
  "ground": "gnd",
  "nets": ["gnd", "vin", "vout"],
  "parts": [
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "p": "vin", "n": "gnd" },
      "voltage": { "value": 5.0, "unit": "V" }
    },
    {
      "name": "r1",
      "kind": "resistor",
      "pins": { "a": "vin", "b": "vout" },
      "resistance": {
        "value": 1000.0,
        "unit": "ohm"
      },
      "ratings": { "power": null }
    },
    {
      "name": "r2",
      "kind": "resistor",
      "pins": { "a": "vout", "b": "gnd" },
      "resistance": { "value": 2000.0, "unit": "ohm" },
      "ratings": { "power": null }
    }
  ]
}
```

## Fields

`version` is `1`. Any number this engine does not read yields only `netlist.version`, even when the rest of the document is valid.

`seed` is an unsigned integer. The manifest records it. It moves switch bounce edges and nothing else. An operating point, and a run whose every `bounce` is `null`, do not depend on it.

`analysis` is `{"op": {}}` or `{"transient": {...}}`, described in [Transient analysis](#transient-analysis). The `op` object must be empty. A string such as `"op"` is `netlist.syntax`.

`ground` names one entry of `nets`. That net is held at 0 V. A name absent from `nets` is `netlist.ground`.

`nets` lists every net name. `parts` lists every part.

`world` is optional. It holds the world quantities the netlist declares, as [World](#world) says.

The engine stores ground first and sorts the other nets by name. It sorts parts by name. Reordering `nets` or `parts` in the JSON does not change observations or diagnostics.

## Transient analysis

`packages/engine/tests/circuits/rc-switch/netlist.json` charges a 1 uF capacitor through a switch and 10 kohm, with a 10 kohm bleeder, then lets it discharge.

```json
"analysis": { "transient": { "stop_ps": 81000000000, "max_step_ps": 20000000, "grid_ps": 20000000 } }
```

| Field | Meaning |
| --- | --- |
| `stop_ps` | The run ends at this time, in picoseconds. |
| `max_step_ps` | No time step is longer, in picoseconds. |
| `grid_ps` | The trace has a row at every multiple of it, in picoseconds. |

All three are JSON integers, like `seed`. They are the scenario's choices, so they carry no unit. A float, a negative number, or a missing field is `netlist.syntax`. `stop_ps` must be greater than 0. `max_step_ps` must be at least 1000 and at most `stop_ps`. `grid_ps` must be at least 1000 and at most `stop_ps`. A break is `netlist.analysis` with the field and the rule.

The rows are t = 0, every multiple of `grid_ps` before `stop_ps`, and `stop_ps`. That is `ceil(stop_ps / grid_ps) + 1` rows. `stop_ps` need not be a multiple of `grid_ps`. The grid never changes the steps, and the longest step is at most `max_step_ps`. A row is a point sample of the solution at its time, solved from the last accepted step and never accepted, as [engine.md](engine.md#rows) says. Between the rows, `envelope.f64` holds each column's minimum and maximum over the two rows and every accepted step between them. [run-folder.md](run-folder.md#traces) says how to read them and how to pick `grid_ps`. A dense switch or source schedule costs steps, not rows, and can hit the run's step limit.

The run solves the operating point at t = 0 first, with every capacitor open or held at its `initial` voltage, every switch at its `initial` contact, and every source at its value at t = 0, which is a waveform's `initial`. That is the initial condition. Then it steps to `stop_ps`. Every switch edge, every source jump, and every potentiometer move before `stop_ps` takes effect at its picosecond. [engine.md](engine.md) describes how the engine steps there, how it finds chip events, and how accurate it is.

## World

The world is the room the circuit sits in. Its quantities are a closed table, extended by a new engine version's rows, never by a netlist.

| Quantity | Unit | Range | A scene's default |
| --- | --- | --- | --- |
| `temperature` | `degC` | -40 to 125 | 25 |
| `humidity` | `%RH` | 0 to 100 | 50 |
| `light` | `lx` | 0 to 100000 | 300 |

`world` maps each quantity the netlist declares to a value object or a `steps` waveform in the quantity's unit, as a potentiometer's `position` is. A value holds at every time. A `steps` waveform holds `initial`, then each event's `level` from its `t_ps` on. A quantity the netlist leaves out has no value, no part may read it, and the run writes no column for it. `{}` declares none, as a missing `world` does.

```json
"world": {
  "temperature": { "value": 25.0, "unit": "degC" },
  "light": {
    "kind": "steps",
    "initial": { "value": 300.0, "unit": "lx" },
    "events": [{ "t_ps": 20000000000, "level": { "value": 50.0, "unit": "lx" } }]
  }
}
```

A name the table lacks is `netlist.world` with rule `unknown`, field `world`. Each value is inside its quantity's range, both ends included. One outside is `netlist.value` with rule `range` on ref `#world.<quantity>`, field `value`, `initial` or `events[k].level`. A different unit is `netlist.unit` on the same ref and field. An event that breaks a schedule rule is `netlist.schedule` on ref `#world.<quantity>` and field `events`. The rules are a `steps` source's: `at_zero`, `not_increasing`, `same_state` and `too_close`, so two changes of one quantity are at least 2000 ps apart. A repeated name, and a `kind` other than `steps`, are `netlist.syntax`.

Every change is a jump, as a `steps` source's is. A row at its time holds the value from before it, and the restart solve there holds the new one. A run writes one trace column per declared quantity, `#world.<quantity>` in its unit, after every part's quantities, and the operating point holds it too. A netlist that declares none writes the trace it wrote before the world existed.

A part whose device reads the world, such as a [`dht22`](#dht22), reads each quantity its device declares. A netlist with such a part must declare those quantities. One it leaves out is `netlist.world` with rule `undeclared` on the part and field `kind`, and the part is dropped. An [`atmega328p`](#atmega328p) reads `temperature` at each sample of its temperature sensor when the netlist declares it, and 25 °C when it does not, so it needs none.

### World-driven fields

A part field that the world drives holds `{"kind": "world", "quantity": ..., "curve": ...}` in place of its value object. The field then has a value on each interval of the quantity's schedule: the curve at the quantity's value there. Its changes are the quantity's changes. A resistor's `resistance` is the one such field so far.

```json
"resistance": {
  "kind": "world",
  "quantity": "light",
  "curve": {
    "kind": "power",
    "x": { "value": 10.0, "unit": "lx" },
    "y": { "value": 10000.0, "unit": "ohm" },
    "exponent": { "value": -1.0, "unit": "1" },
    "max": { "value": 1000000.0, "unit": "ohm" }
  }
}
```

A curve reads the quantity's value `w`. `x`, `x0` and `x1` are in the quantity's unit, and `y`, `y0`, `y1` and `max` in the field's.

| `kind` | Value | Fields | For |
| --- | --- | --- | --- |
| `power` | `y * (w / x)^exponent`, at most `max` | `x`, `y`, `exponent` in `1`, `max` | a light-dependent resistor, whose dark resistance is `max` |
| `beta` | `y * exp(beta * (1 / (w + 273.15) - 1 / (x + 273.15)))` | `x`, `y`, `beta` in `K` | an NTC thermistor's B equation, on `temperature` only |
| `linear` | the line through `(x0, y0)` and `(x1, y1)` | `x0`, `y0`, `x1`, `y1` | a sensor with a linear output, such as an LM35's 10 mV per degC |

Each curve is monotonic in `w` over the range of a quantity it may read, so its values over that range lie between its values at the range's two ends. A `beta` or `linear` curve is monotonic for every `w` it may read. A `power` curve is monotonic only where `w / x` is not negative, so it reads only a quantity whose range does not go below 0, `light` or `humidity`. At `w` 0 a negative exponent makes `y * (w / x)^exponent` infinite, and the curve is `max` there, the dark resistance of a light-dependent resistor. Any other value above `max` is `max` too. A value that is not a number is never capped. Parse checks the field's own rule at both ends, so every value a schedule or a session can set keeps it. An end that breaks it is `netlist.value` on field `<field>.curve at <end> <unit>`, such as `resistance.curve at 0.0 lx`, with the field's rule, or rule `finite` when the curve is not finite there, such as a `power` curve with `y` 0 and a negative exponent at 0 lx. A `power` curve on `temperature` is `netlist.world` with rule `non_negative` on field `<field>.curve.kind`. A quantity the netlist does not declare is `netlist.world` with rule `undeclared` on field `<field>.quantity`, and a name the table lacks is rule `unknown` there. A `beta` curve on another quantity is `netlist.world` with rule `temperature` on field `<field>.curve.kind`. A `power` curve's `x` at or below 0 is `netlist.value` with rule `positive`. A `linear` curve whose `x1` equals its `x0` is `netlist.value` with rule `distinct`. A unit that is not the one the table above names is `netlist.unit` on field `<field>.curve.<name>`. An unknown `kind`, a missing field and an unknown field are `netlist.syntax`.

## Names

A name is an ASCII letter, then ASCII letters, digits, or `_`, from 1 to 64 bytes.

A part named `net` is `netlist.name` with problem `reserved`. Pin, unit, and value checks on that part are still reported. A name that fails the grammar is `netlist.name` and those checks are not run, because the part has no ref. A net may be named `net`.

Two nets, or two parts, with the same name are `netlist.duplicate`. The ref is `#net.<name>` or `#<part>`. Pin, unit, and value checks on a duplicate part are still reported.

## Parts

`kind` selects the part. Pin keys must be exactly that kind's pin names. A missing or extra key is `netlist.pins`. An extra key that is a legal name and names a net absent from `nets` is also `netlist.unknown_net`.

### Resistor

`kind` is `resistor`. Pins are `a` and `b`, in that order for the sign rule.

`resistance.value` is in `ohm`, greater than 0, and its reciprocal must be finite. `resistance` may instead be a world-driven field, as [World-driven fields](#world-driven-fields) says, which keeps the same rule at both ends of its quantity's range. Zero or a negative value is `netlist.value` with rule `positive`. A positive value whose reciprocal overflows is `netlist.value` with rule `finite_reciprocal`. A different unit is `netlist.unit`. A number outside the f64 range is `netlist.syntax`.

`ratings` holds `power`. See [Ratings](#ratings).

### Potentiometer

`kind` is `potentiometer`. Pins are `a`, `wiper`, and `b`, in that order. `a` to `wiper` is `position * resistance`. `wiper` to `b` is `(1 - position) * resistance`.

`resistance` follows the resistor's rules, unit `ohm`. `position` is a value in `1`, greater than 0 and less than 1, or a `steps` waveform in `1`, described in [Source waveforms](#source-waveforms). A value at 0 or 1, or outside that range, is `netlist.value` with rule `fraction` on field `position`. A `steps` level that breaks it uses that level's field, such as `position.initial` or `position.events[0].level`. A different unit is `netlist.unit` on that field. A `kind` of `step` or `pulse`, or any other kind, is `netlist.syntax`.

A `steps` position follows the `steps` schedule rules on field `position.events`. An event moves the wiper, and the row at that time holds the position from before it.

Each leg must be positive, and its reciprocal must be finite, the same check as `resistance`. A leg that breaks it is `netlist.value` on field `resistance`, and `value` is that leg.

A potentiometer has no `ratings` field. It has no `i`, `v`, or `p`. Both legs are DC paths.

### Voltage source

`kind` is `voltage_source`. Pins are `p` and `n`. The source sets `V(p) - V(n)` equal to `voltage`.

`voltage` is a value in `V`, which may be any finite number, including 0 and negative values. Or it is a waveform in `V`, described in [Source waveforms](#source-waveforms). A different unit is `netlist.unit`.

`p` and `n` must be on two nets. A source with both on one net is `net.short`, an error, because no solve can hold one net apart from itself.

`ratings` is optional. When present it holds `current`, the most the supply delivers, which bounds `|i|`. A netlist without the object leaves the source unrated, as one written before sources had ratings does. See [Ratings](#ratings).

### Current source

`kind` is `current_source`. Pins are `p` and `n`. The source's `current` leaves it at `p`, flows through the circuit, and returns at `n`. `packages/engine/tests/circuits/i-divider` drives 1 mA into 1 kohm parallel to 2 kohm.

```json
{
  "name": "i1",
  "kind": "current_source",
  "pins": { "p": "vt", "n": "gnd" },
  "current": { "value": 0.001, "unit": "A" }
}
```

`current` is a value in `A`, any finite number, or a waveform in `A`. A different unit is `netlist.unit`. Source stepping scales it, as it scales a voltage source.

A current source has no `ratings` field. It joins no DC path. A net reached only through current sources and capacitors without an `initial` voltage is `net.floating`, and only gmin holds it: 1 mA into such a net reads 1e9 V. A resistor to ground closes the loop.

### Source waveforms

A source's `voltage` or `current` is a value object, which holds at every time, or an object with `kind`, which changes. Its levels are values in the source's unit, any finite number. Its times are JSON integers in picoseconds, like `stop_ps`. They are the scenario's choices, so they carry no unit.

A step holds `initial` until `at_ps`, then `final`.

```json
"voltage": {
  "kind": "step",
  "initial": { "value": 0.0, "unit": "V" },
  "final": { "value": 5.0, "unit": "V" },
  "at_ps": 1000000000
}
```

A `steps` waveform holds `initial`, then each event's `level` from its `t_ps` on. `events` may be empty, and then the waveform holds `initial` at every time, the same as a value object. `packages/engine/tests/circuits/v-steps` steps from 0 V to 5 V at 99.5 ns and to 1 V at 9.9995 us.

```json
"voltage": {
  "kind": "steps",
  "initial": { "value": 0.0, "unit": "V" },
  "events": [
    { "t_ps": 2000, "level": { "value": 5.0, "unit": "V" } },
    { "t_ps": 4000, "level": { "value": 1.0, "unit": "V" } }
  ]
}
```

An event that breaks a schedule rule is `netlist.schedule` on field `voltage.events` or `current.events`. The rules are the switch's, and a level has no bounce window. `same_state` means the event's SI value equals the level in force. `too_close` means the event is less than 2000 ps after the previous one. `overflow` does not fire: adding no window to a time cannot pass the largest time.

A pulse holds `initial` until `delay_ps`, then `pulsed` for `width_ps`, then `initial` until the next period, every `period_ps`. `packages/engine/tests/circuits/current-pulse` uses this one. A single pulse is one whose next rise is past `stop_ps`.

```json
"current": {
  "kind": "pulse",
  "initial": { "value": 0.0002, "unit": "A" },
  "pulsed": { "value": 0.001, "unit": "A" },
  "delay_ps": 1000000000,
  "width_ps": 2000000000,
  "period_ps": 5000000000
}
```

A `serial` waveform holds `mark` while idle and for a 1 bit, and `space` for a start bit and a 0 bit. Each event sends `bytes` back to back from `t_ps`, in frame `8N1`, at `baud` bits per second. `packages/engine/tests/circuits/v-serial` sends the byte 49 at 9600 baud from 1 ms. `packages/engine/tests/circuits/atmega-serial-echo` is the chip example that uses this source.

```json
"voltage": {
  "kind": "serial",
  "mark": { "value": 5.0, "unit": "V" },
  "space": { "value": 0.0, "unit": "V" },
  "events": [
    { "t_ps": 1000000000, "baud": 9600, "frame": "8N1", "bytes": [49] }
  ]
}
```

Bit boundary `i` of an event is `t_ps + floor(i * 10^12 / baud)` picoseconds, computed in 128-bit arithmetic. Boundary 0 is the event's `t_ps`. Frame `j` of the event occupies boundaries `10j` through `10j + 10`. Bit 0 of a frame is the start bit, at `space`. Bits 1 to 8 are the data byte, least significant bit first, `mark` for a 1 and `space` for a 0. Bit 9 is the stop bit, at `mark`. The line holds `mark` at t = 0 and between events. A jump is drawn only where the level changes, so two equal bits in a row are one held level, and the frame end is not a jump. The stop bit and the idle line are both `mark`.

An event that breaks a schedule rule is `netlist.schedule` on field `voltage.events` or `current.events`.

- `at_zero`: `t_ps` is 0.
- `not_increasing`: `t_ps` is not after the previous event's `t_ps`.
- `same_state`: `bytes` is empty, so the event does not change the level.
- `overlap`: `t_ps` is before the previous event ends. The message names that end in picoseconds. An event that starts exactly when the previous one ends is legal.
- `overflow`: the event's end does not fit in an unsigned 64-bit integer. Later events of that source are not checked. An event whose end does not fit is `overflow` even when its `t_ps` is also before the previous end, and `overlap` is not also reported.

`baud` above 500000000 is `netlist.waveform` with rule `too_close` on field `voltage.events[k].baud` or `current.events[k].baud`. A bit at 500000000 baud is 2000 ps, and a shorter bit is under the gap two jumps of one source must keep. `mark` and `space` with the same SI value are `netlist.waveform` with rule `equal` on field `voltage.space` or `current.space`. The message says the two levels are equal, and `data.value` is 0. A `baud` of 0, a byte above 255, a float `baud`, and a `frame` other than `8N1` are `netlist.syntax`.

Every change is an ideal jump. The level on the interval that ends at a jump is the old one, so a trace row at the time of a jump holds it. A jump at `stop_ps` leaves the last row on the old level. The operating point holds `initial` for a `step`, a `pulse`, or `steps`, and `mark` for `serial`. An operating point run reads only that.

A `step` or `pulse` time that breaks a rule is `netlist.waveform`, with its dotted field, such as `current.width_ps`, and the rule. Every broken rule is reported.

- `at_zero`: `at_ps` or `delay_ps` is 0. The operating point already holds `initial` there.
- `too_close`: `width_ps` is under 2000, or `period_ps` is under `width_ps` plus 2000. Two jumps of one source are at least 2 ns apart, as two edges of one switch are.
- `overflow`: `delay_ps` plus `width_ps` does not fit in an unsigned 64-bit integer.

A `step` or a `pulse` jump between equal levels is still a jump. A `serial` waveform whose `mark` and `space` are equal is `netlist.waveform` with `equal`. A `steps` event whose level equals the level in force is `netlist.schedule` with `same_state`, and so is a `serial` event with no bytes. A missing or unknown key, a float or negative time, a `kind` other than `step`, `pulse`, `steps`, or `serial`, and an object with both `value` and `kind` are `netlist.syntax`. A level in the wrong unit is `netlist.unit` with its dotted field, such as `current.pulsed` or `voltage.events[2].level`. `steps` times and `serial` event times are `netlist.schedule`, not `netlist.waveform`.

### Capacitor

`kind` is `capacitor`. Pins are `a` and `b`.

`capacitance.value` is in `F`. It follows the resistance rules. It must be greater than 0, and its reciprocal must be finite.

`series_resistance` is optional. When present, `series_resistance.value` is in `ohm` and follows the resistance rules. It must be greater than 0, and its reciprocal must be finite. It runs from `a` to an internal node, and the capacitance runs from that node to `b`. The internal node is not observed. `#c1.v` is still `V(a) - V(b)`, and `#c1.i` is the current entering `a`. A missing key leaves the capacitance between `a` and `b`, with no internal node, which is the ideal capacitor. A value of 0 or below is `netlist.value` with rule `positive` on `series_resistance`. A wrong unit is `netlist.unit` with that field.

`initial` is required. It is a value in `V`, any finite number, or `null`.

```json
"initial": { "value": 0.0, "unit": "V" }
```

A capacitor with `"initial": null` is open at the operating point, so `#c1.i` is 0 there. A capacitor with a value is held there, behind a series resistance of 1 fs divided by its capacitance. That resistance is the backward Euler companion of a 1 fs step. With no `series_resistance`, `V(a) - V(b)` is the value plus `#c1.i` times that hold resistance, and `#c1.i` is the current the hold takes. With `series_resistance`, the hold is the voltage across the capacitance, and `V(a) - V(b)` is that voltage plus `#c1.i` times the series resistance and the hold resistance. For 1 uF and 5 mA the held voltage moves by 5 pV. A held capacitor across a voltage source, held capacitors in parallel, and held capacitors in series across a source then solve instead of making the matrix singular. Capacitors in parallel share their current by capacitance. Two in parallel at different values meet at their capacitance-weighted mean, as their charge would. With no `series_resistance`, a held value that disagrees with a voltage source across it draws the difference over the hold resistance, 1e9 A per volt for 1 uF. With one, it draws the difference over the series resistance plus the hold resistance. That current also enters the error estimate of the first steps, so they fall to the 1 ps floor and grow back. It is no rating sample, since no part carries it, as [diagnostics.md](diagnostics.md#ratingcurrent) says. A missing key is `netlist.syntax`. A wrong unit is `netlist.unit` with field `initial`. In a step the current follows from the voltage across the capacitance and the last accepted point.

`ratings` is optional. When present it holds `reverse_voltage`, in `V`: how far `V(a) - V(b)` may sit below 0. Past that, pin `a` warns [`rating.voltage`](diagnostics.md#ratingvoltage), measured over pin `b`. A voltage exactly on the floor is inside. A value of 0 warns on any reverse voltage, and a pin sitting on pin `b` is inside. A negative value is `netlist.value` with rule `non_negative`. A netlist without the object leaves the capacitor unrated, as one written before capacitors had a reverse rating does. `"reverse_voltage": null` is deliberately unrated. A forward voltage has no rating. A missing key inside a present object is `netlist.syntax`.

### Inductor

`kind` is `inductor`. Pins are `a` and `b`.

`inductance.value` is in `H` and `series_resistance.value` is in `ohm`. Each follows the resistance rules. It must be greater than 0, and its reciprocal must be finite.

```json
"inductance": { "value": 0.505, "unit": "H" },
"series_resistance": { "value": 70.0, "unit": "ohm" }
```

The series resistance runs from `a` to an internal node, and the inductance runs from that node to `b`. The internal node is not observed. A coil's winding resistance is its series resistance. The resistance must be greater than 0, so an ideal inductor is a small resistance. At the operating point the inductance is a short, and the part is its series resistance. An inductor has no `initial`. Its current at t = 0 is the operating point's. In a step the current follows from the voltage across the inductance and the last accepted point. `packages/engine/tests/circuits/rl-step` steps 5 V across 0.505 H and 70 ohm.

An inductor has no `ratings` field.

### Relay

`kind` is `relay`. Pins are `coil_a`, `coil_b`, `com`, `no` and `nc`. It is a coil and one changeover contact.

```json
{
  "name": "k1",
  "kind": "relay",
  "pins": { "coil_a": "vcc", "coil_b": "coil", "com": "vcc", "no": "no", "nc": "nc" },
  "inductance": { "value": 0.505, "unit": "H" },
  "coil_resistance": { "value": 70.0, "unit": "ohm" },
  "pull_in_current": { "value": 0.05357142857142857, "unit": "A" },
  "drop_out_current": { "value": 0.007142857142857143, "unit": "A" },
  "on_resistance": { "value": 0.1, "unit": "ohm" },
  "off_resistance": { "value": 100000000.0, "unit": "ohm" }
}
```

`inductance.value` is in `H`. `coil_resistance.value`, `on_resistance.value` and `off_resistance.value` are in `ohm`. `pull_in_current.value` and `drop_out_current.value` are in `A`. Each follows the resistance rules. It must be greater than 0, and its reciprocal must be finite. `drop_out_current` must also be below `pull_in_current`. At or above it is rule `hysteresis` on `drop_out_current`.

The coil is an inductor from `coil_a` to `coil_b`, with `coil_resistance` as its series resistance, as [Inductor](#inductor) describes. The contact joins `com` to `no` through `on_resistance` while it is closed and through `off_resistance` while it is open. It joins `com` to `nc` the other way round. A normally open relay leaves `nc` on a net of its own. The contact closes when the magnitude of the coil current rises above `pull_in_current`, and it opens when the magnitude falls to `drop_out_current` or below. A current of either sign closes it. The engine reads the current across the coil resistance and locates each crossing to the picosecond, as [Chip events](engine.md#chip-events) says. The contact moves there, and the coil current is held across the move.

The contact starts open, and the operating point closes it when the coil current at t = 0 is above `pull_in_current`. A coil current between the two starts open. A contact that moves its own coil current from above pull-in to drop-out or below has no state that agrees with its solve. When its `nc` feeds its own coil and the coil is powered at t = 0, the operating point reports `chip.unsettled`. When the coil is powered later, it holds its current across each move and the contact buzzes.

The model has no armature travel and no contact bounce. Its operate time is the coil current's rise to pull-in, and its release time is the current's fall to drop-out. A real relay adds the armature's travel, a few milliseconds, and its contact bounces. A coil released with no diode across it opens the contact as soon as its current collapses, where a relay sheet gives a release time of milliseconds. The inductance does not change with the armature's position. There is no contact rating, no coil temperature, and no arcing. Breaking a coil current through the contact puts that current times `off_resistance` across the coil.

`packages/engine/tests/circuits/relay-switch` is the 5 V high-sensitivity coil of a common 10 A hobby relay. Its sheet gives 70 ohm, a pull-in voltage of at most 75 % of 5 V and a drop-out voltage of at least 10 %, so pull-in is 53.6 mA and drop-out 7.14 mA. The sheet gives no inductance. At 70 ohm with no travel, 0.504943 H reaches pull-in in exactly its 10 ms operate time, and the example rounds that to 0.505 H. A switch grounds the coil at 2 ms and opens at 30 ms. A diode across the coil then carries its current. The contact lights an LED from 12.017 ms to 41.320 ms.

`packages/engine/tests/circuits/relay-flyback` uses that coil, diode and LED, and a 2N3904 in place of the switch. A pin is high from 5 ms to 30 ms and drives the base through 1 kohm, so the transistor sinks the coil. When the pin falls, the collector rises to 5.723 V, the supply plus 0.723 V across the diode, and the diode's current peaks at 65.7 mA. The coil current decays through the diode. The contact closes at 15.927 ms and opens at 40.932 ms, after the pin has fallen. The run accepts 1340 steps after t = 0.

A relay has no `ratings` field. It has `i`, `v` and `p` of its coil, as [Sign rules](#sign-rules) says. Its logic signal is `contact`, described in [Refs](#refs). It is a DC path from `coil_a` to `coil_b`, from `com` to `no`, and from `com` to `nc`, in either state.

### Switch

`kind` is `switch`. Pins are `a` and `b`. It is a resistor whose value follows a contact over time.

```json
{
  "name": "sw1",
  "kind": "switch",
  "pins": { "a": "vbtn", "b": "gnd" },
  "on_resistance": { "value": 0.1, "unit": "ohm" },
  "off_resistance": { "value": 1000000000.0, "unit": "ohm" },
  "initial": "open",
  "events": [
    { "t_ps": 2000000000, "state": "closed" },
    { "t_ps": 12000000000, "state": "open" }
  ],
  "bounce": {
    "window": { "value": 1000000000, "unit": "ps" },
    "max_bounces": { "value": 4, "unit": "1" }
  }
}
```

`on_resistance` applies while the contact is `closed`, and `off_resistance` while it is `open`. Both are in `ohm` and follow the resistance rules.

`initial` is `"open"` or `"closed"`. It is the contact at the operating point.

`events` lists contact changes. Each is `{"t_ps": <integer>, "state": "open" | "closed"}`. The list may be empty. An event that breaks a rule is `netlist.schedule` with its index and the rule.

- `at_zero`: `t_ps` is 0. The operating point already holds `initial` there.
- `same_state`: `state` equals the contact before it.
- `not_increasing`: `t_ps` is not after the previous event's `t_ps`.
- `too_close`: `t_ps` is less than 2000 ps after the event before it. With bounce, it is less than 2000 ps after that event's `t_ps` plus `window`.
- `overflow`: `t_ps` plus `window` does not fit in an unsigned 64-bit integer. Later events of that switch are not checked.

Edges of one switch are always at least 2 ns apart. The oracle's control ramps are 1 ns long, so they never overlap.

`bounce` is `null`, or adds seeded contact bounce after every event. `window` is in `ps`. `max_bounces` is a count in `1`, from 1 to 255. Both take a JSON integer and a source. A float, or a count above 255, is `netlist.syntax`. A value of 0 is `netlist.value` with rule `positive`. `window` must be at least `2 * max_bounces * 2000` ps, room for every drawn edge 2 ns apart. Less is `netlist.value` with rule `bounce_room`.

Bounce is a fixed, integer-only expansion. Event `k` of switch `name` draws from SplitMix64 seeded with `mix(mix(seed ^ fnv1a64(name)) ^ k)`, where `mix` is SplitMix64's output function and `fnv1a64` is 64-bit FNV-1a over the name's bytes. It draws a count `n` from 1 to `max_bounces`, then `2n` values from 0 to `window - 2n * 2000`, sorted. Draw `j`, counted from 1, plus `j * 2000` is the offset of the `j`-th flip after the event. So the contact flips `2n` times within `window` and ends at the event's state. A draw below `m` is the high 64 bits of a 64-bit output times `m`. `packages/engine/scripts/ngspice_refs.py` implements the same steps. The offsets are uniform, which is enough to test exact breakpoints and determinism. They are not a contact physics model.

The contact on the interval that ends at time `t` is the state after every edge strictly before `t`. A trace row at the time of an edge therefore holds the old contact. An edge at `stop_ps` leaves the last row on the old contact. `observations.json` lists every edge at its picosecond.

A switch has no `ratings` field. A switch is a DC path in either state, because its off resistance is finite.

A detent knob whose two lines each pull low on their own is two switches. On a round display board's knob, each detent pulled exactly one of its two lines low and then released it, and the two lines were never low at the same time. So that knob is not a quadrature encoder. Over two captures sampled about every 100 us, line A was low for 51.2 ms to 130.4 ms over 13 detents and line B for 29.4 ms to 331.7 ms over 18, and the idle time from one release to the next press was 41.7 ms to 9185.6 ms, each to 0.1 ms. Those times are board observations, and they follow how fast a hand turned the knob. Model each line as a `switch` from its pin to `gnd`, beside a 10 kohm pull-up from the pin to the supply. The 10 kohm is read from the board's schematic, R52 and R53, not measured. A detent is one switch's `closed` event and its `open` event the low time later, with `bounce` `null`. The board did not show which line turns clockwise, so the lines are A and B. Swapping them is swapping the two switches' pins. In a session a detent is two contact inputs on one switch, closed at its time and open at its time plus the low time. An input may ask for a time after now, so both can go in before the detent starts. `packages/engine/tests/circuits/atmega-board-knob` is this model.

### Diode and LED

`kind` is `diode` or `led`. Both take the same fields and solve the same equation. Pins are `anode` and `cathode`.

The part is a junction in series with a resistance. The junction current is `Is * (exp(vj / (N * kT/q)) - 1) + GMIN * vj` down to `vj = -3 * N * kT/q`. Below that it is `-Is * (1 + (3 * N * kT/q / (e * vj))^3) + GMIN * vj`, ngspice-42's reverse law with no breakdown voltage. The two meet at `-3 * N * kT/q` in current and slope. `GMIN` is 1e-12 S across the junction, from the internal node to the cathode, where ngspice-42 puts `CKTgmin`. The engine puts no node gmin on that internal node. `V(anode) - V(cathode)` is `vj` plus the current times `Rs`.

| Field | Symbol | Unit |
| --- | --- | --- |
| `saturation_current` | `Is` | `A` |
| `emission_coefficient` | `N` | `1` |
| `series_resistance` | `Rs` | `ohm` |

Each value must be greater than 0, and its reciprocal must be finite. Those are the resistor's rules. `n*VT` must also be a normal number, and `vcrit` must be finite and positive. `VT` is `k*T/q` at 298.15 K with the exact SI values of `k` and `q`. `vcrit` is `n*VT*ln(n*VT/(sqrt(2)*Is))`, which is positive when `n*VT > sqrt(2)*Is`. A junction that breaks either condition is `netlist.value` with rule `finite_junction`. The field is `emission_coefficient` when `n*VT` is not normal. It is also `emission_coefficient` when `vcrit` is not finite and positive and `Is` is a normal number. The field is `saturation_current` when `vcrit` is not finite and positive and `Is` is not a normal number.

The device is at 25 C, 298.15 K. `k` and `q` are the exact SI values. There is no temperature field and no temperature scaling. A value fitted at another temperature does not describe the part here.

`ratings` holds `current` and `power`.

`packages/engine/tests/circuits/led-5v` and `packages/engine/tests/circuits/led-9v` use a Kingbright WP7113ID, a red 5 mm LED. Its `Is`, `N`, and `Rs` solve the equation exactly through three points read off the datasheet's forward current figure at 25 C: (1.70 V, 2 mA), (1.90 V, 10 mA), and (2.00 V, 20 mA). The fit gives `Is` = 6.711e-10 A, `N` = 4.427, and `Rs` = 2.115 ohm. Each value's `at` names the figure, the points, and the temperature, so the fit can be redone from the citation.

### Zener

`kind` is `zener`. Pins are `anode` and `cathode`.

The part is two junctions. `series_resistance` joins the anode to internal node 0, and the forward junction runs from there to the cathode, with the diode's law and the forward `Is` and `N`. `breakdown_resistance` joins internal node 1 to the cathode, and the breakdown junction runs from internal node 1 to internal node 0. Breakdown current does not flow in the forward junction. Each junction has its own `GMIN`, 1e-12 S, across it. The engine puts no node gmin on either internal node. `i` is the current through `series_resistance`, entering the anode. In breakdown `i` and `v` are negative.

| Field | Symbol | Unit |
| --- | --- | --- |
| `saturation_current` | `Is` | `A` |
| `emission_coefficient` | `N` | `1` |
| `series_resistance` | `Rs` | `ohm` |
| `zener_voltage` | `VZ` | `V` |
| `zener_current` | `IZT` | `A` |
| `zener_impedance` | `ZZT` | `ohm` |
| `breakdown_resistance` | `Rb` | `ohm` |

`VZ` is the pin voltage at `IZT` in breakdown. `ZZT` is the slope of that pin voltage at `IZT`. A datasheet column headed at 1 kHz is used here as the DC slope. Each of the seven values must be greater than 0, and its reciprocal must be finite. `zener_impedance` must exceed `series_resistance + breakdown_resistance`. Less or equal is `netlist.value` with rule `above_resistance` on `zener_impedance`. `zener_voltage` must exceed `zener_current * (series_resistance + breakdown_resistance)`. Less or equal is `netlist.value` with rule `above_drop` on `zener_voltage`. When either inequality fails, the part does not also report `finite_junction`.

When both hold, the engine derives the breakdown junction:

```text
nbvt = (ZZT - Rs - Rb) * IZT
bvj  = VZ - IZT * (Rs + Rb)
Isb  = IZT / expm1(bvj / nbvt)
```

`expm1` comes from `libm`. At `IZT` the pin voltage is `VZ`. The slope there is `Rs + Rb + nbvt / (IZT + Isb)`, which equals `ZZT` when `Isb` is negligible beside `IZT`. `nbvt` must be normal. `Isb` must be normal and positive, and the breakdown junction's `vcrit` must be finite and positive. A break is `finite_junction`. It names `zener_impedance` when `nbvt` is not normal, and `zener_voltage` when `Isb` or `vcrit` is not. The forward junction follows the diode's `finite_junction` rule on its own fields.

The device is at 25 C, 298.15 K. There is no temperature field, no `VZ` temperature coefficient, and no junction capacitance. A thermal-equilibrium reading taken at another temperature does not describe the part here.

`ratings` holds `current` and `power`. `current` bounds `|i|` in both directions. That is stricter than a sheet that states a forward drop at a current above the reverse current limit.

`packages/engine/scripts/fit_zener.py` fits the 1N4733A row. `Is`, `N` and `Rs` solve the forward equation through three stroke centers of Vishay 85816 Rev. 2.7, p3 Fig. 2, at Tj = 25 C: (0.622 V, 0.1 mA), (0.695 V, 1 mA), and (0.876 V, 100 mA). `VZ`, `IZT` and `ZZT` are that document's p2 row, 5.1 V, 49 mA and 7 ohm. `Rb` is the value that puts 1 mA on the 4.46 V read of EIC 1N4727A to 1N4761A Fig. 1 at TA = 25 C. The script prints `Is` = 2.908000103814407e-13 A, `N` = 1.2314970051462708, `Rs` = 0.35129980768421137 ohm and `Rb` = 4.5183447909672836 ohm. The forward drop at 200 mA is 0.932671 V, under the 1.2 V maximum. The slope at 49 mA is 7 ohm. Against the figure, the model is 4.460 V at 1 mA, 4.6475 V at 5 mA (1.695% high), 4.7442 V at 10 mA (1.807% high) and 5.100 V at 49 mA (0.394% above the 5.08 V read). The impedance at 1 mA is 109.2571 ohm, under the 550 ohm maximum. At 1 V reverse the current entering the anode is -2.2907918471752469e-12 A. The total includes both junctions and `GMIN`, and the reverse-current maximum is 10 uA. The reverse curve below 1 mA is an extrapolation, since no point under 1 mA was fit. Current in a circuit biased below the knee is likely under-predicted. The examples are the 5.1 V part.

`packages/engine/tests/circuits/zener-shunt` is 9 V through 100 ohm into the zener, with a 1 kohm load. The cathode sits at 4.996963715824736 V and the zener current is -0.03503339912093115 A. `packages/engine/tests/circuits/zener-sweep` steps a pin from -2 V to 12 V in 0.25 V levels, one per millisecond, through 220 ohm into the cathode, and accepts 190 steps after t = 0. The rows pass through forward, reverse below the knee, the knee and breakdown.

### Linear regulator

`kind` is `linear_regulator`. Pins are `in`, `out` and `gnd`.

The part is one branch. `i` is the pass current from `in` to `out`. `vi` is `V(in) - V(gnd)` and `vo` is `V(out) - V(gnd)`.

```text
vset = Vout + k * (vi - Vout)
a    = vset - Ro * i
raw  = vi - Vdo - Rdo * i
b    = -Rf * i + s * sp((raw + Rf * i) / s)
vo   = smin(a, b) - Rlim * d * sp((i - Ilim) / d) + Rblk * db * sp(-i / db)
ig   = (Iq + r * i) * h(vi / Vdo) + Iq * (vi / Vdo) * h(-vi / Vdo)
```

`sp(x)` is `ln(1 + e^x)`, written as `x + ln(1 + e^-x)` when `x` is positive, so the exponential's argument stays at or below 0. `smin(a, b)` is `a - s * sp((a - b) / s)`. `b` is that same smooth maximum of `raw` and `-Rf * i`. `h(x)` is 0 at and below 0, `3x^2 - 2x^3` from 0 to 1, and 1 at and above 1. `ln` and `exp` come from `libm`. The second term of `ig` is 0 for `vi` at least 0, derivative included, so a circuit whose input stays up is unchanged. Below 0 it loads the input. It is `-Iq` at `vi` = `-Vdo`, and `Iq * vi / Vdo` further below. That current leaves pin `in`, so a series diode's leakage holds a reversed input near 0 V instead of letting the leak set the pin to the source.

`s` is 1e-3 V, the width of the smooth maximum and of the dropout knee. `Rf` is 0.25 ohm. `db/di` stays between `-Rf` and `-Rdo`, and `db/dvi` stays between 0 and 1. Below the ground pin the softplus of `raw` alone underflows to 0, and the floor keeps a slope of `-Rf`. With the input at the ground pin and no load, `b` is 0. A load that draws pass current `i` then sits near `-Rf * i`, which is below the pin. A current source of 0.5 A from `out` to `gnd`, with the input held at 0 V, solves at -0.12499999999996891 V. A 10 us trapezoidal run of `packages/engine/tests/circuits/regulator-load-step` stops when the floor is 0.15 ohm, at 1.21482e-09 s on `vx_u1#branch`, and finishes when the floor is 0.2 ohm. A floor of 0.25 ohm finishes that run, the same deck with 100 uF on the output, the same deck stepping the input to 4 V, and the same deck with a 1 ohm dropout resistance. `d` is `1e-3 * Ilim`. `Rlim` is `100 * Vout / Ilim`, so a short sits about 1% above `Ilim`. `Rblk` is 1e9 ohm and `db` is 1e-12 A, so a reverse current stays under a nanoamp per volt. These numbers are constants of the law. They are not fields.

`iin` is `i + ig`, the current entering `in`. `iout` is `-i`, the current entering `out`. The ground pin takes `-ig`. `p` is `vi * iin + vo * iout`.

The stamp reads the last solution, because the row is the law linearised at that current and that `vi`. The first linearisation, from a zero start, is not the operating point, and the solve iterates on it. The part has no junction, so it takes no `pnjlim` and no limiting. Convergence compares the solved `vo` with the law at the solved `i` and `vi`.

`in` joins `gnd`, and `out` joins `gnd`. Both are DC paths. A capacitor on the input and a capacitor on the output, with nothing else, are not `net.floating`.

| Field | Symbol | Unit |
| --- | --- | --- |
| `output_voltage` | `Vout` | `V` |
| `line_regulation` | `k` | `1` |
| `output_resistance` | `Ro` | `ohm` |
| `dropout_voltage` | `Vdo` | `V` |
| `dropout_resistance` | `Rdo` | `ohm` |
| `current_limit` | `Ilim` | `A` |
| `quiescent_current` | `Iq` | `A` |
| `ground_current_ratio` | `r` | `1` |

`output_voltage`, `output_resistance`, `dropout_voltage`, `dropout_resistance` and `current_limit` must be greater than 0, and each reciprocal must be finite. `line_regulation` must be at least 0 and below 1. A value outside that is `netlist.value` with rule `below_one`. `quiescent_current` and `ground_current_ratio` must be at least 0. A negative value is `netlist.value` with rule `non_negative`.

`ratings` holds `current` and `power`. `current` bounds `|iout|`.

The law is the DC table. It has no internal pole and no output inductance. It has no enable pin. Pin `in` warns [`rating.voltage`](diagnostics.md#ratingvoltage) below -0.3 V over `gnd`. LP2985, TI SLVS522, states that floor. The AMS1117 sheet this repo cites states a 15 V maximum and no negative floor, and the SGM2212 sheet states 22 V and no negative floor, so -0.3 V is the model's floor for every `linear_regulator`, the margin a chip pin uses below its ground. A pin exactly on -0.3 V is inside. It has no diode from `out` to `in`. A circuit that needs that diode adds a `diode` part. The current limit is flat. There is no thermal shutdown, and a `power` rating warns instead. `ig` follows `h` while `vi` is positive and the reverse term while `vi` is negative, and it does not add a further rise in dropout. There is no minimum load. Ripple from the input reaches the output through the static `k`.

`packages/engine/scripts/fit_regulator.py` fits the LM1117-3.3 row of TI LM1117, SNOS412Q. `output_voltage` is 3.3 V, the p5 typical at 10 mA and VIN 5 V. `line_regulation` is `0.001 / 10.25` = 9.75609756097561e-05, the p5 typical of 1 mV from 4.75 V to 15 V at 0 mA. `output_resistance` is `0.001 / 0.8` = 0.00125 ohm, the p5 typical of 1 mV from 0 to 800 mA at VIN 4.75 V. `dropout_resistance` is `0.1 / 0.7` = 0.14285714285714288 ohm and `dropout_voltage` is 1.0857142857142859 V, the line through the p6 typicals of 1.10 V at 100 mA and 1.20 V at 800 mA. `current_limit` is 1.2 A, the p6 typical at a 5 V differential. `quiescent_current` is 0.005 A, the p6 LM1117-3.3 typical. The maximum on that row is 10 mA, at VIN of at most 15 V, and the model uses the typical. `ground_current_ratio` is 0. The p6 table gives no load term, and the model holds the ground current at 5 mA wherever `h` is 1. `ratings.current` is 0.8 A, the p1 output current, which is also the top of the p5 load-regulation range from 0 to 800 mA. `ratings.power` is `(125 - 25) / 61.6` = 1.6233766233766234 W. Section 7.3's recommended junction limit is 125 C and section 7.4's SOT-223 RθJA is 61.6 C/W. The ambient in that figure is 25 C. Section 7.1's absolute junction limit is 150 C, which would be 2.029220779220779 W. The rating uses 125 C.

Note 3 on p5 measures line and load regulation at constant junction room temperature. The fit uses that table. Figure 7-1 at 25 C reads about 1.0 V at no load and about 1.17 V at 800 mA. Figure 7-3 reads about -0.2% for an 800 mA step at 25 C, which is 6.6 mV at 3.3 V. The figures do not name the version.

The script's law, with the output's gmin carrying the no-load current, gives 3.3001533536585366 V at 10 mA and 5 V, inside the p5 window of 3.267 V to 3.333 V. Line regulation from 4.75 V to 15 V is 0.0009999644533134067 V. Load regulation from no load to 800 mA at 4.75 V is 0.0010362129392489372 V, 36 uV over the 1 mV typical, inside 0.1 mV. From 100 mA to 800 mA it is 0.0008749999999997371 V. Dropout by note 4, a 100 mV drop from the value at 4.8 V, is 1.1 V at 100 mA, 1.1571428571428575 V at 500 mA and 1.2000000000000002 V at 800 mA. The 500 mA point is the one the line does not use. The table says 1.15 V, so the model is 0.007142857142857562 V high. A short at a 5 V differential draws 1.2119950393315033 A, inside the p6 window of 0.8 A to 1.5 A. The ground current at 5 V is 0.005 A.

Figure 7-8 is captioned for the LM1117-5.0. It prints CIN and COUT of 10 uF tantalum, VIN of 6.5 V, a 0.1 A preload, and a step from 0.1 A to 0.5 A. The dip read from that figure for the ratio is 0.14 V. This model's 0.5 A step on 10 uF moves the output by `Ro` times the step, 0.000625 V. The ratio of that reading to the model's step is 224.00000000000003, and 280.0 per ampere. The gap is 2.35 orders of magnitude. The 0.14 V depth, the recovery, and an inductance read off that dip are readings of the 5.0 V figure.

The AMS1117 states dropout at a 1% drop. At 3.3 V that is 33 mV. SNOS412Q note 4 is a 100 mV drop. The two 1.1 V numbers are different measurements. This fit uses the 100 mV definition, and it does not fit the AMS1117.

`packages/engine/tests/circuits/regulator-3v3` is 5 V into 33 ohm. The output sits at 3.300040852111104 V, `#u1.iout` is -0.10000123794606076 A and `#u1.iin` is 0.10500123794606077 A. `packages/engine/tests/circuits/regulator-dropout` steps the input from 0 V to 6 V in 0.1 V levels, one per millisecond, into 33 ohm. The rows pass through off, dropout, the knee and regulation. At 12 ms the input is 1.1 V and `#net.vout` is 0.014224138525133514 V, which is 5.798777846788283e-10 V above the unfloored reference 0.01422413794525573 V. At 13 ms the input is 1.2 V and `#net.vout` is 0.11379310344825949 V, which is 1.421500417375654e-11 V below the unfloored reference 0.1137931034624745 V. `packages/engine/tests/circuits/regulator-load-step` steps 5 V on at 1 ns, with 10 uF on the output, a 330 ohm preload, and a switch that adds 6.6 ohm from 1 ms to 3 ms. It accepts 289 steps after t = 0. The same capacitor, preload and switch, fed by an ideal 3.3 V source that is already at 3.3 V, accepts 80. The charge from 0 to 40 us takes 38 of the 289, and the window from 28 us to 40 us takes 5. The 100 us after the two load edges take 178 together. Two steps last 1 ns. Between 40 us and the first load edge the largest reversal of `#net.vout` is 1.2221068601547813e-10 V. Ten times `output_resistance` accepts 274 steps, and a tenth accepts 533. The charge current is about 1.21 A, so the run warns `rating.current` and `rating.power`. The fixture `regulator-c100u` is that circuit with 100 uF on the output and the load step's `grid_ps` of 10 us. `regulator-step-4v` is that circuit with the input stepping to 4 V and `grid_ps` of 20 us. `regulator-rdo-1` is that circuit with `dropout_resistance` of 1 ohm, `dropout_voltage` of 0.1 V and `grid_ps` of 10 us. Their references use the same trapezoidal options as the load step.

The board fragment uses a second fit of the same law, the legacy standard-grade LP2985-33 of TI SLVS522S. Page 30 Table 8-1 allows that orderable part to be the legacy chip or the new chip. The fit uses the legacy column because the revision 3 schematic predates the new chip. `packages/engine/scripts/fit_regulator.py lp2985` computes it. The floor constants are the ones above. `output_voltage` is 3.3 V, page 30 Table 8-1, where orderable code 33 means 3.3 V. `line_regulation` is 0.000231, from the page 5 legacy typical of 0.007 %/V over the 11.7 V from 4.3 V to 16.0 V, times 3.3 V. `output_resistance` is 0.3322147651006711 ohm. The page 5 standard-grade window widens by 0.049499999999999995 V from 0.001 A to 0.15 A, and 0.149 A is that span. The printed tolerances are 0.015 and 0.03. That window runs to the rated output current, so the fit uses this slope. The same reading of the 1 mA to 50 mA window, tolerances 0.015 and 0.025 from 0.001 A to 0.050 A, is 0.673469387755102 ohm. Page 25 Figure 7-3 reads about 0.26 ohm and Figure 7-5 reads about 0.29 ohm. Those figures are legacy load steps. This field is the 150 mA window's slope. A typical-part reading would come from the figures. `dropout_resistance` is 1.8322147651006713 ohm and `dropout_voltage` is 0.0051677852348993285 V, the line through the page 6 legacy typicals of 0.007 V at 0.001 A and 0.28 V at 0.15 A. `current_limit` is 0.35 A, the page 7 legacy peak typical. `quiescent_current` is 6.5e-05 A, the page 6 legacy typical at 0 A. `ground_current_ratio` is 0.005233333333333334, through 6.5e-05 A at 0 A and 0.00085 A at 0.15 A. `ratings.current` is 0.15 A, the p4 section 5.3 maximum. `ratings.power` is 0.48685491723466406 W, `(125.0 - 25.0) / 205.4`. Section 5.3's recommended junction limit is 125.0 C and section 5.4's legacy DBV RθJA is 205.4 C/W. The absolute junction limit of 150.0 C would be 0.6085686465433301 W. The rating uses 125.0 C.

At 0.001 A and 4.3 V the law gives 3.299898785234899 V, inside 3.2504999999999997 V to 3.3494999999999995 V. Unloaded line regulation from 4.3 V to 16.0 V is 0.002702604015081711 V, against 0.0027026999999999997 V. Load regulation from 0.001 A to 0.15 A at 4.3 V is 0.0495000000000001 V. Dropout by note 1 on page 7 is 0.007000000000000117 V at 0.001 A and 0.28000000000000025 V at 0.15 A. The points the line does not use are 0.004474638054340119 V at 0 A against 0.001 V, 0.023489932885906395 V at 0.01 A against 0.04 V, and 0.09677852348993321 V at 0.05 A against 0.12 V. A short at 5 V draws 0.3533758822109669 A. The page 7 legacy short typical is 0.4 A, and the law's headroom cannot match it. Ground current at 5 V is 6.5e-05 A at 0 A and 0.00085 A at 0.15 A. It is 7.023333333333332e-05 A at 0.001 A against 7.5e-05 A, 0.00011733333333333333 A at 0.01 A against 0.00012 A, and 0.00032666666666666673 A at 0.05 A against 0.00035 A. An open output at 5 V sits at 3.3004289040043036 V. An open output at 0 V sits at 0.0006984790072844244 V.

The law cannot match the page 7 legacy ripple rejection of 45 dB at 1000 Hz, the output noise of 30 uVrms, or the shutdown ground current of 0.01 uA. The ON/OFF high typical is 1.4 V. The board ties that pin to the input, and the model has no enable. The legacy minimum output capacitor is 2.2 uF. The schematic's C3 is 1 uF. The law has no stability condition. Section 7.1.4 on page 21 allows the output 0.3 V above the input. Reverse current in the body diode is not limited, and the law blocks it. The ground-current ramp reaches its full value at `dropout_voltage`, 0.0051677852348993285 V. The legacy chip is specified from 2.2 V. The new chip's UVLO, 2.2 V typical and 2.4 V maximum rising, is not modelled, and the legacy column has none.

### Op-amp

`kind` is `op_amp`. Pins are `in_p`, `in_n`, `out`, `vcc` and `gnd`. One part is one channel. A dual part, such as `parts/lmv358-msop8` or `parts/lm358-dip8`, maps two bodies, `a` and `b`, onto the same `vcc` and `gnd`.

The part is a behavioural element with one pole. It has one internal node, the integrator `x`, with a capacitor `C` of 1 nF from `x` to `gnd`, and two branches. Branch 0 is that capacitor's current. Branch 1 is the output current `i`, which leaves the part at `out`, so `i` is positive while the part sources. `vd` is `V(in_p) - V(in_n)`, `u` is `V(x) - V(gnd)`, `s` is `V(vcc) - V(gnd)` and `vo` is `V(out) - V(gnd)`. `sp` and `ln` are as in [Linear regulator](#linear-regulator), and `sg(x)` is `1 / (1 + e^-x)`.

```text
C du/dt = Is * tanh(gm * vd / Is) - (u - s/2) / R - G * kc * (sp((u - s - 0.1) / kc) - sp((-0.1 - u) / kc))
Is      = SR * C
gm      = 2 * pi * GBW * C
R       = A0 / gm
t       = lo + k * sp((u - lo) / k) - k * sp((u - hi) / k),   lo = Vsl,  hi = s - Vsh
vo      = t - Ro * i - Rl * d * sp((i - Isc) / d) + Rl * d * sp((-i - Isc) / d)
```

The integrator slews at `SR` while `|vd|` is large, and its unity-gain frequency is `GBW`, because `gm / (2 * pi * C)` is `GBW`. `R` to mid-supply sets the DC gain to `A0`. `C` cancels from every answer. `G` is `1e7 * C` siemens and `kc` is 0.01 V, a soft clamp that holds `u` within about 0.1 V of each rail. While the slew current runs into the clamp, `u` sits `0.1 + kc * ln(exp(SR / 1e5 V/s) - 1)` past the rail, 0.2 V at 1 V/us. `k` is 1e-3 V, the knee of the swing. `d` is `1e-3 * Isc` and `Rl` is `500 V / Isc`, so a short sits a little past `Isc`.

The output current is drawn from `vcc` when the part sources and returned to `gnd` when it sinks: `vcc` gives `i * sg(i / 1 uA)` and `gnd` gives the rest. A bias current of `Ib * sg((V(vcc) - 0.8 - V(input)) / 0.05)` leaves each input, drawn from `vcc`, so it turns off as the input comes within 0.8 V of `vcc`. That 0.8 V is a constant of the law. It is the LMV358's typical point where the bias current turns off, and it is not `input_headroom`. A conductance of `Icc / 5 V` joins `vcc` to `gnd`, so the supply current is `Icc` at a 5 V supply and scales with it. Every current enters and leaves at a pin. These numbers are constants of the law. They are not fields.

The stamp reads the last solution, as the regulator's does. Convergence compares the solved `vo` with the law at the solved `u`, `s` and `i`. The internal node takes no node gmin.

`out` joins `vcc` and `gnd`, and `vcc` joins `gnd`. These are DC paths. An input is not: a net reached only through an input is `net.floating`.

| Field | Symbol | Unit |
| --- | --- | --- |
| `gain_bandwidth` | `GBW` | `Hz` |
| `slew_rate` | `SR` | `V/s` |
| `open_loop_gain` | `A0` | `1` |
| `input_bias_current` | `Ib` | `A` |
| `swing_high` | `Vsh` | `V` |
| `swing_low` | `Vsl` | `V` |
| `output_resistance` | `Ro` | `ohm` |
| `short_circuit_current` | `Isc` | `A` |
| `supply_current` | `Icc` | `A` |
| `input_headroom` | none | `V` |

`gain_bandwidth`, `slew_rate`, `open_loop_gain`, `output_resistance` and `short_circuit_current` must be greater than 0, and each reciprocal must be finite. Zero or a negative value is `netlist.value` with rule `positive`. At exactly 0 ohm the output row has no current slope below the limit, so Newton's matrix is singular when another source fixes the output. Any positive resistance gives that row a slope. `input_bias_current`, `swing_high`, `swing_low`, `supply_current` and `input_headroom` must be at least 0. A negative value is `netlist.value` with rule `non_negative`.

An op-amp has no quantities and no `ratings` object. A positive `input_headroom` is checked as [`rating.voltage`](diagnostics.md#ratingvoltage) on `in_p` and `in_n`. The ceiling is `V(vcc) - input_headroom`, measured over the part's `gnd`, and there is no floor. A pin exactly on the ceiling is inside. Zero rates no ceiling. The differential stage does not read the field, so the pin voltages stay what the law above gives. Read the output current from what the output drives.

Not modelled: input offset voltage, CMRR, PSRR and noise. A second internal pole and the phase margin: the law has one pole, so the part does not peak on its own. `Ro` and a capacitor on the output add a pole outside the part. A follower into 200 pF does not ring, and into 100 nF it rings near 230 kHz. Overdrive recovery past the clamp, and supply current that follows the output current beyond the split above. Temperature. The input stage does not collapse above `input_headroom`: the law keeps comparing, and the run warns. On an LM358, TI SLOS068AB section 7.3.3, page 26, says the output phase is correct while at least one input stays in the common-mode range, and undefined when both inputs are outside it. The warning is per input, so one input above the ceiling warns even when the other is inside. An input more than 0.3 V below the negative rail, where that section limits the input current to 1 mA and calls the phase undefined, is not checked. One output resistance covers sourcing and sinking alike, so a load returned above ground sees the sourcing slope while sinking.

`packages/engine/tests/circuits/op-amp-*` run the LMV358's electrical values, the typicals of TI SLOS263Y p7 section 6.8 at VCC 5 V, and set `input_headroom` to 0. Zero rates no ceiling, so those runs stay free of `rating.voltage`. The ramp reaches the supply and the step that reverses its current carries the node a little past it. A ceiling at the rail would warn on that step. `parts/lm358-dip8` carries 1.5 V, the SLOS068AB p10 section 5.7 maximum of `(V+) - 1.5 V` for the LM358 at 25 °C, and `parts/lmv358-msop8` carries 0.8 V, which is 5 V minus the section 6.8 typical upper end of 4.2 V. That row's guaranteed upper end is 4 V. `packages/engine/tests/circuit/runs/op_amp.rs` holds each circuit to a band from that table and to the law above. `op-amp-bias` is a follower with `in_p` behind 1 Mohm from 2.5 V, and a second with `in_p` behind 1 Mohm from 4.95 V, where the bias current is off. `op-amp-swing` overdrives four channels into 10 kohm and 2 kohm to 2.5 V. `op-amp-slew` steps a follower from 0.5 V to 4.0 V and back. `op-amp-gain-10k` and `op-amp-gain-200k` drive a gain of 10 with a 64-level staircase of a 10 mV sine. `op-amp-short` holds one output at 0 V and one at 5 V. `op-amp-ramp`, `op-amp-square` and `op-amp-hysteresis` are comparators against 2.5 V, driven by a 500 V/s ramp, a 10 kHz square and the ramp with 10 kohm in and 100 kohm of feedback. `packages/engine/tests/circuit/runs/lm358.rs` runs the same circuits on the values in `parts/lm358-dip8/part.jsonc`, the LM358 column of TI SLOS068AB p10 section 5.7, and returns the swing loads to `gnd`.

### NPN and PNP transistors

`kind` is `npn` or `pnp`. Both take the same fields and solve the same equations. The kind is the polarity. Pins are `collector`, `base` and `emitter`, in SPICE order.

The part is ngspice's Gummel-Poon bipolar transistor at its defaults, which is the transport form of Ebers-Moll with a forward Early voltage. `RB`, `RC` and `RE` join the base, collector and emitter pins to internal nodes `b'`, `c'` and `e'`. The junction voltages are inside them. `vbe = s * (V(b') - V(e'))` and `vbc = s * (V(b') - V(c'))`, where `s` is +1 for `npn` and -1 for `pnp`. `VT` is the diode's.

```text
ibe = IS * (exp(vbe / (NF*VT)) - 1)
ibc = IS * (exp(vbc / (NR*VT)) - 1)
ic  = (ibe - ibc) * (1 - vbc/VAF) - ibc/BR - GMIN*vbc
ib  = ibe/BF + ibc/BR + GMIN*(vbe + vbc)
```

Below `vbe = -3 * NF*VT`, `ibe` is `-IS * (1 + (3 * NF*VT / (e * vbe))^3)` in place of the exponential, and below `vbc = -3 * NR*VT`, `ibc` is `-IS * (1 + (3 * NR*VT / (e * vbc))^3)`. That is the diode's reverse law, which ngspice-42's `bjtload` takes on each junction. `s * ic` enters `c'` and `s * ib` enters `b'`. `GMIN` is 1e-12 S across each junction, where ngspice puts it. `CJE` is a constant capacitance from `b'` to `e'`, and `CJC` one from `b'` to `c'`. The step's error control holds both as it holds a capacitor part.

| Field | Symbol | ngspice | Unit |
| --- | --- | --- | --- |
| `saturation_current` | `IS` | `is` | `A` |
| `forward_beta` | `BF` | `bf` | `1` |
| `reverse_beta` | `BR` | `br` | `1` |
| `forward_emission` | `NF` | `nf` | `1` |
| `reverse_emission` | `NR` | `nr` | `1` |
| `early_voltage` | `VAF` | `vaf` | `V` |
| `base_resistance` | `RB` | `rb` | `ohm` |
| `collector_resistance` | `RC` | `rc` | `ohm` |
| `emitter_resistance` | `RE` | `re` | `ohm` |
| `base_emitter_capacitance` | `CJE` | `cje` with `mje=0` | `F` |
| `base_collector_capacitance` | `CJC` | `cjc` with `mjc=0` | `F` |

Each value must be greater than 0, and its reciprocal must be finite. Those are the resistor's rules. A PNP takes the same positive values as an NPN. `saturation_current` with `forward_emission`, and `saturation_current` with `reverse_emission`, must each keep the diode's `finite_junction` rule. A pair that breaks it names `forward_emission` or `reverse_emission` where the diode names `emission_coefficient`. When both pairs break it on `saturation_current`, that field is reported once.

`ratings` holds `current` and `power`. `current` bounds `|ic|`.

The model leaves out these effects.

- Beta does not fall at high or low current. 2N3903/D Rev. 8 Figure 15 shows hFE at 100 mA at about 0.45 of its peak, so a 100 mA switch needs about twice the base current the model says. A design that saturates with the datasheet's minimum hFE still saturates here.
- There is no reverse Early voltage. Only reverse-active operation sees it.
- The capacitances do not depend on voltage, and there is no transit time. So a saturated switch has no storage time and turns off early, by up to 200 ns for the 2N3904 row of 2N3903/D and 225 ns for 2N3906/D. At MCU and relay timescales that does not show.
- The device is at 25 C, as the diode is.
- A junction does not break down. A b-e junction reversed past `VEBO`, 6 V for the 2N3904 and 5 V for the 2N3906, conducts nothing more, and `VCEO` is not checked.

The engine's 1e-12 S from every net to ground is not on `c'`, `b'` and `e'`. As in ngspice-42, whose default substrate junction is vertical for an `npn` and lateral for a `pnp`, `GMIN` runs from ground to `c'` in an `npn` and to `b'` in a `pnp`. With the junction `GMIN`, an off NPN with 5 V on its collector takes 10 pA into the collector, as ngspice's does, and an off PNP draws what ngspice's does from its emitter supply.

Eight examples use a small-signal NPN or a small-signal PNP. `packages/engine/tests/circuits/relay-flyback` is one of them. `packages/engine/tests/circuits/npn-led` drives the base from a 5 V pin through 1 kohm and saturates the transistor, which sinks the LED of `packages/engine/tests/circuits/led-5v` through 220 ohm. `packages/engine/tests/circuits/pnp-load` pulls a PNP's base to 0 V through 1 kohm, against a 10 kohm pull-up, and the PNP feeds 100 ohm from 5 V. `packages/engine/tests/circuits/ce-amp` is a common-emitter stage at 12 V, biased by 47 kohm and 10 kohm, with 4.7 kohm on the collector and 1 kohm on the emitter. It sits in the active region, where the Early voltage sets the collector current's slope.

`packages/engine/tests/circuits/npn-sweep` steps a pin from 0 V to 5 V in 50 levels of 0.1 V, one each millisecond, through 4.7 kohm into the base of an NPN with 70 ohm from the collector to 5 V. The collector stays at 5 V in cut-off up to a 0.5 V pin, then falls in a straight line through the active region. From a 2.6 V pin the transistor saturates, and the collector flattens out. More base current no longer moves it, and `ic / ib` falls from 173 to 75. The collector is 0.40 V at 2.7 V and 0.32 V at 5 V.

`packages/engine/tests/circuits/pnp-sweep` steps a pin from 5 V down to 0 V in those 50 levels, through 10 kohm, and the PNP feeds 100 ohm from 5 V. The collector stays near 0 V while the pin is high, then climbs and stops about 0.23 V below the rail. 0.01 ohm sits between the 5 V source and the emitter. It drops under 0.5 mV, and it is there for the ngspice reference, which aborts its first step with the emitter on the ideal source.

`packages/engine/tests/circuits/npn-pulse` drives a 1 kHz pin through 10 kohm into a saturating NPN, with 1 kohm and 100 nF on the collector, and has a row every microsecond. The pin rises at 200 us. The transistor sinks about 74 mA, beta times the base current, so the collector falls in a straight line of about 0.7 V each microsecond, from 5 V at 200 us to 0.22 V at 207 us. The pin falls at 700 us, and 1 kohm charges 100 nF back up, from 10% at 711 us to 90% at 931 us.

`packages/engine/tests/circuits/npn-blinker` is two of those NPNs as an astable. Each collector has 470 ohm and the LED of `packages/engine/tests/circuits/led-5v`. Each base has 47 kohm from 5 V. The 10 uF capacitor runs from the first collector to the second base, and the 4.7 uF capacitor runs from the second collector to the first base. The supply steps from 0 V to 5 V at 1 ns. Both LEDs are off at the first row. Both light for about 9 ms, then take turns. The LED on the first collector stays on for about 271 ms and the LED on the second for about 125 ms, and that pair repeats about every 410 ms. The first turn of the first LED is shorter, about 245 ms. Both LED currents stay above 1 mA for about 4.5 ms at one handoff and about 9.5 ms at the other. The capacitors differ so the on times differ and the first LED leads on every run. With equal capacitors the rounding of the solve chooses which LED leads. `packages/engine/tests/circuit/runs/blinker.rs` reads the on times and the 410 ms from the first collector's falling edges.

The sweeps hold each level for a millisecond, so every row is a settled DC point. They show the DC curve, and the device capacitances carry no current on their rows. `packages/engine/tests/circuits/npn-pulse`, `packages/engine/tests/circuits/npn-blinker`, and the transistor fixtures in [reference.md](reference.md) are the circuits whose rows depend on the capacitances.

The NPN is the 2N3904 row of 2N3903/D Rev. 8, and the PNP is 2N3906/D Rev. 4. `NF` and `NR` are 1. `BF` is 173.2, the geometric mean of the guaranteed hFE of 100 to 300 at 10 mA and 1 V, because the typical gain figure is normalised. `VAF` is 100 V, which puts `hoe` at 9.1 umhos at 1 mA and 10 V, inside both datasheets' ranges. `CJE` and `CJC` are the datasheets' maximum `Cibo` and `Cobo`. `packages/engine/scripts/fit_transistor.py` fits `IS`, `BR`, `RB`, `RC` and `RE` from each datasheet's "ON" voltages figure at 25 C. `BR` comes from `VCE(sat)` at 1 mA, `IS` and `RE` from `VBE` at 1 V at 1 mA and 100 mA, `RB` from `VBE(sat)` at 100 mA, and `RC` from `VCE(sat)` at 100 mA, iterated to a fixed point. It prints the model's voltages at 10 mA and 50 mA beside the readings and the table limits. Each model value is inside the table limits. Figure 15 of 2N3906/D draws 1 to 10 mA narrower than 10 to 100 mA, so the PNP fit reads its points at the labelled 1 mA and 100 mA lines, and its `VBE` at 10 mA is 0.747 V against the curve's 0.728 V. Each value's `at` or `note` names the row or figure and the readings.

```json
{
  "name": "q1",
  "kind": "npn",
  "pins": { "collector": "c", "base": "b", "emitter": "gnd" },
  "saturation_current": { "value": 8.095e-15, "unit": "A" },
  "forward_beta": { "value": 173.2, "unit": "1" },
  "reverse_beta": { "value": 0.2704, "unit": "1" },
  "forward_emission": { "value": 1.0, "unit": "1" },
  "reverse_emission": { "value": 1.0, "unit": "1" },
  "early_voltage": { "value": 100.0, "unit": "V" },
  "base_resistance": { "value": 5.707, "unit": "ohm" },
  "collector_resistance": { "value": 1.531, "unit": "ohm" },
  "emitter_resistance": { "value": 0.867, "unit": "ohm" },
  "base_emitter_capacitance": { "value": 8e-12, "unit": "F" },
  "base_collector_capacitance": { "value": 4e-12, "unit": "F" },
  "ratings": { "current": null, "power": null }
}
```

### N-channel MOSFET

`kind` is `nmos`. Pins are `drain`, `gate` and `source`. The bulk is the source pin.

The part is ngspice's level 1 MOSFET with the bulk on the source, no body diode, and `W/L` folded into `KP`. `vgs` is `V(gate) - V(source)` and `vds` is `V(drain) - V(source)`. For `vds >= 0`:

```text
cutoff     vgs - VTO <= 0     id = 0
saturation vds >= vgs - VTO   id = KP/2 * (vgs - VTO)^2 * (1 + LAMBDA*vds)
linear     otherwise          id = KP * vds * (vgs - VTO - vds/2) * (1 + LAMBDA*vds)
```

For `vds < 0` the drain and source swap roles, so `id = -f(vgs - vds, -vds)`, where `f` is the table above. `GMIN * vds` is then added to `id`. `GMIN` is 1e-12 S. The gate draws no DC current.

`CGS`, `CGD` and `CDS` are constant capacitances from gate to source, gate to drain, and drain to source. The step's error control holds each one as it holds a capacitor part. The part has no internal node.

| Field | Symbol | Unit |
| --- | --- | --- |
| `transconductance_parameter` | `KP` | `A/V^2` |
| `threshold_voltage` | `VTO` | `V` |
| `channel_length_modulation` | `LAMBDA` | `1/V` |
| `gate_source_capacitance` | `CGS` | `F` |
| `gate_drain_capacitance` | `CGD` | `F` |
| `drain_source_capacitance` | `CDS` | `F` |

`transconductance_parameter` and the three capacitances must be greater than 0, and each reciprocal must be finite. Those are the resistor's rules. `threshold_voltage` must be finite, and it may be negative or zero. `channel_length_modulation` must be at least 0. A wrong unit on it reports `netlist.unit` and does not also report `non_negative`.

`ratings` holds `current` and `power`. `current` bounds `|id|`.

The model leaves out these effects.

- The body diode. A discrete MOSFET conducts from source to drain when the drain goes below the source. This part does not. A circuit that needs that path adds a `diode` from source to drain.
- Body effect. The bulk is the source pin.
- Series resistance at the drain and the source. The fit folds it into `KP`.
- Subthreshold conduction. Below `VTO` the current is `GMIN * vds`.
- Mobility degradation. The square law fitted at 4 V and 5 V gives 1.86 A at `VGS` = 10 V, where Figure 1 shows about 1.5 A.
- Voltage-dependent capacitances. The gate charge is approximate.

The part has no internal node. The engine's 1e-12 S from every net to ground is on the drain, gate and source nets. The device's own `GMIN` is only from drain to source, which is where ngspice-42 leaves it when `is` is 0 and the bulk is the source. An off device with 5 V from drain to source takes 5 pA into the drain.

`packages/engine/tests/circuits/nmos-led` is a 5 V pin through 100 ohm driving a small-signal N-channel MOSFET that sinks the LED of `packages/engine/tests/circuits/led-5v` and 560 ohm from 12 V. The gate is at 5 V, inside the fit. A 3.3 V gate overstates the drain current. The fit gives 0.115 A at 3.3 V, where Figure 2 reads about 0.07 A. The model conducts from 1.08 V. Figure 2's typical curve starts near 2.5 V. At a 2.5 V gate the model gives about 47 mA, where a typical part passes almost nothing.

`packages/engine/tests/circuits/nmos-sweep` steps a pin from 0 V to 5 V in 50 levels of 0.1 V, through 100 ohm into the gate, with 150 ohm from the drain to 12 V. The drain stays at 12 V below the 1.08 V threshold, the drain current then follows the square law, and the drain voltage falls to about 0.45 V once the MOSFET can sink the load. It is a DC sweep like the BJT ones. The `nmos-pulse` fixture is the circuit whose rows depend on the gate capacitances.

The values are from 2N7000/D Rev. 5. `VTO` is 1.08 V and `KP` is 0.0468 A/V^2. The netlist rounds the two-point solution through Figure 2 at 25 C at (4.0 V, 0.20 A) and (5.0 V, 0.36 A) to those two values. The rounded values give 0.1995 A and 0.3596 A at those gates. `VTO` sits inside the table's 0.8 V to 3.0 V. `LAMBDA` is 0, because Figure 1 draws flat saturation lines. `rDS(on)` at `VGS` = 4.5 V is 6.25 ohm by the model, against the table's 6.0 ohm maximum at 75 mA. `gm` at 200 mA is 0.137 S. The ON CHARACTERISTICS row prints `gfs` as 100 µmhos at `VDS` = 10 V and `ID` = 200 mA. The glyph is a Symbol mu. 2N7000 Rev. C1 states 100 millimhos at those biases, and NDS7002A/D Rev. 11 lists the 2N7000 row as `gFS` minimum 100 mS. Those sheets state 0.1 S. The model is not refit. Its `gm` at 200 mA is above that 0.1 S minimum. `CGD` is the maximum `Crss`, 5 pF. `CGS` is the maximum `Ciss` minus that, 55 pF. `CDS` is the maximum `Coss` minus that, 20 pF. The current rating is 0.2 A and the power rating is 0.35 W, the table's continuous drain current and total dissipation at TC = 25 C.

```json
{
  "name": "m1",
  "kind": "nmos",
  "pins": { "drain": "d", "gate": "g", "source": "gnd" },
  "transconductance_parameter": { "value": 0.0468, "unit": "A/V^2" },
  "threshold_voltage": { "value": 1.08, "unit": "V" },
  "channel_length_modulation": { "value": 0.0, "unit": "1/V" },
  "gate_source_capacitance": { "value": 55e-12, "unit": "F" },
  "gate_drain_capacitance": { "value": 5e-12, "unit": "F" },
  "drain_source_capacitance": { "value": 20e-12, "unit": "F" },
  "ratings": { "current": null, "power": null }
}
```

### P-channel MOSFET

`kind` is `pmos`. Pins, fields, ratings and the equations are the `nmos` ones, and every value stays positive, `threshold_voltage` included. The device is that `nmos` with each terminal voltage and the drain current negated, so it conducts when `V(gate) - V(source)` is below `-VTO`. `id` is negative when the channel sources current into a load tied to ground.

`packages/engine/tests/circuits/pmos-high-side` is a 5 V source, a P-channel MOSFET and 10 ohm to ground. The gate sits at 5 V, then 0 V, then 5 V. While it is on, `#m1.id` is negative and `#m1.p` is positive. `packages/engine/tests/circuits/pmos-sweep` steps that gate from 5 V down to 0 V. The drain stays near ground until `|Vgs|` passes `VTO`, then rises toward the source. Both use the FDN340P fit from `parts/pmos-fdn340p-sot23`.

### NE555

`kind` is `ne555`. Pins are `gnd`, `trig`, `out`, `reset`, `cont`, `thres`, `disch` and `vcc`, in package order. `packages/engine/tests/circuits/ne555-led` is the astable of TI SLFS022K Figure 6-5, 10 kohm and 47 kohm with 10 uF, driving an LED through 220 ohm. Its values cite SLFS022K, and its ladder cites the schematic of the pin-compatible LM555, TI SNAS548D.

```json
{
  "name": "u1",
  "kind": "ne555",
  "pins": {
    "gnd": "gnd", "trig": "thr", "out": "out", "reset": "vcc",
    "cont": "cont", "thres": "thr", "disch": "dis", "vcc": "vcc"
  },
  "ladder_resistance": { "value": 5000.0, "unit": "ohm" },
  "reset_threshold": { "value": 0.7, "unit": "V" },
  "discharge": {
    "on_resistance": { "value": 18.75, "unit": "ohm" },
    "off_resistance": { "value": 250000000.0, "unit": "ohm" }
  },
  "supply_resistance": { "value": 1875.0, "unit": "ohm" },
  "output": {
    "high_drop": { "value": 0.9, "unit": "V" },
    "high_resistance": { "value": 8.0, "unit": "ohm" },
    "low_drop": { "value": 0.016666666666666666, "unit": "V" },
    "low_resistance": { "value": 16.666666666666668, "unit": "ohm" }
  }
}
```

The model is the ladder, three comparators and SLFS022K Table 6-1.

- Three `ladder_resistance` resistors run from VCC to CONT, from CONT to an internal lower node, and from that node to GND. CONT is the pin, so a source or a load on it moves both thresholds. The lower node is not observed.
- TRIG reads high above the lower node, THRES above CONT, and RESET above GND by more than `reset_threshold`. Each is a watch, described in [Chip events](engine.md#chip-events). No comparator has an offset, hysteresis, or input current.
- RESET low resets the latch. Otherwise TRIG low sets it. Otherwise THRES high resets it. Otherwise it holds. The table is read on levels, not edges.
- OUT follows the latch. Set, OUT is `output.high_drop` under VCC behind `output.high_resistance`. Reset, OUT is `output.low_drop` over GND behind `output.low_resistance`. Its current flows to the chip's own VCC and GND pins. The drops are sources, so source stepping scales them.
- DISCH has `discharge.on_resistance` to GND while the latch is reset, and `discharge.off_resistance` while it is set.
- `supply_resistance` runs from VCC to GND, the chip's quiescent current.
- There is no propagation delay.

| Field | Unit | Rule |
| --- | --- | --- |
| `ladder_resistance`, `supply_resistance` | `ohm` | the resistor's |
| `discharge.on_resistance`, `discharge.off_resistance` | `ohm` | the resistor's |
| `output.high_resistance`, `output.low_resistance` | `ohm` | the resistor's |
| `reset_threshold` | `V` | greater than 0, else rule `positive` |
| `output.high_drop`, `output.low_drop` | `V` | at least 0, else rule `non_negative` |

A break is `netlist.value` with the dotted field, such as `discharge.off_resistance`. A wrong unit is `netlist.unit` with the same field.

The latch's state is the chip's only state. It is not a netlist field. The operating point settles it, as described in [Chip events](engine.md#chip-events).

A chip has no `ratings` field and no `i`, `v`, or `p`. Its logic levels are refs of their own, described in [Refs](#refs). It is a DC path from VCC to CONT, CONT to GND, VCC to GND, OUT to VCC, OUT to GND, and DISCH to GND, in either latch state. A net reached only through TRIG, THRES, or RESET floats.

### ATmega328P

`kind` is `atmega328p`, an ATmega328P running firmware, Microchip DS40002061B. Pins are the 28-pin DIP's, named as Figure 1-1 prints them and listed in package order: `PC6`, `PD0` to `PD4`, `VCC`, `GND_8`, `PB6`, `PB7`, `PD5` to `PD7`, `PB0` to `PB5`, `AVCC`, `AREF`, `GND_22`, and `PC0` to `PC5`, then `ADC6` and `ADC7`. The two ground pins carry their pin numbers, because a pin name is unique. The DIP's 28 are required, so an unused one sits on a net of its own. `ADC6` and `ADC7` are the TQFP's two inputs of the ADC alone (§1.1.9), with no port behind them, so they join nothing at DC. A part may leave either out, as the DIP has neither, and the ADC reads 0 V on a channel whose pin the part leaves out. `GND_8` and `GND_22` name one net: Figure 1-1 names both pins `GND`, and §1.1.2 gives that pin one function, ground. A part that puts them on two nets is `netlist.same_net` on its `GND_22` pin.

This document runs `packages/engine/tests/firmware/toggle.S` at 16 MHz into 1 kohm on PB5. `cx1` and `cx2` are the crystal's load capacitors and `cref` decouples `AREF`. Each is held at 0 V at the operating point, which gives its net a DC path.

```json
{
  "version": 1,
  "seed": 0,
  "analysis": { "transient": { "stop_ps": 20000000, "max_step_ps": 1000000, "grid_ps": 1000000 } },
  "ground": "gnd",
  "nets": [
    "gnd", "aref", "pb0", "pb1", "pb2", "pb3", "pb4", "pb5", "pc0", "pc1", "pc2", "pc3", "pc4",
    "pc5", "pc6", "pd0", "pd1", "pd2", "pd3", "pd4", "pd5", "pd6", "pd7", "vcc", "xtal1",
    "xtal2"
  ],
  "parts": [
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "p": "vcc", "n": "gnd" },
      "voltage": { "value": 5.0, "unit": "V" }
    },
    {
      "name": "r1",
      "kind": "resistor",
      "pins": { "a": "pb5", "b": "gnd" },
      "resistance": { "value": 1000.0, "unit": "ohm" },
      "ratings": { "power": null }
    },
    {
      "name": "cx1",
      "kind": "capacitor",
      "pins": { "a": "xtal1", "b": "gnd" },
      "capacitance": { "value": 2.2e-11, "unit": "F" },
      "initial": { "value": 0.0, "unit": "V" }
    },
    {
      "name": "cx2",
      "kind": "capacitor",
      "pins": { "a": "xtal2", "b": "gnd" },
      "capacitance": { "value": 2.2e-11, "unit": "F" },
      "initial": { "value": 0.0, "unit": "V" }
    },
    {
      "name": "cref",
      "kind": "capacitor",
      "pins": { "a": "aref", "b": "gnd" },
      "capacitance": { "value": 1e-07, "unit": "F" },
      "initial": { "value": 0.0, "unit": "V" }
    },
    {
      "name": "u1",
      "kind": "atmega328p",
      "pins": {
        "PC6": "pc6", "PD0": "pd0", "PD1": "pd1", "PD2": "pd2", "PD3": "pd3", "PD4": "pd4",
        "VCC": "vcc", "GND_8": "gnd", "PB6": "xtal1", "PB7": "xtal2", "PD5": "pd5",
        "PD6": "pd6", "PD7": "pd7", "PB0": "pb0", "PB1": "pb1", "PB2": "pb2", "PB3": "pb3",
        "PB4": "pb4", "PB5": "pb5", "AVCC": "vcc", "AREF": "aref", "GND_22": "gnd",
        "PC0": "pc0", "PC1": "pc1", "PC2": "pc2", "PC3": "pc3", "PC4": "pc4", "PC5": "pc5"
      },
      "firmware": ":10000000259A1D9A0FE10A95F1F700000000000003\n:02001000F8CF27\n:00000001FF\n",
      "clock_hz": { "value": 16000000, "unit": "Hz" },
      "fuses": { "low": 255, "high": 217, "extended": 253 },
      "output": {
        "high_drop": {
          "value": 0.0, "unit": "V"
        },
        "high_resistance": {
          "value": 25.0, "unit": "ohm"
        },
        "low_drop": {
          "value": 0.0, "unit": "V"
        },
        "low_resistance": {
          "value": 25.0, "unit": "ohm"
        }
      },
      "pull_up_resistance": {
        "value": 35714.0, "unit": "ohm"
      },
      "reset_pull_up_resistance": {
        "value": 46296.0, "unit": "ohm"
      },
      "leakage_resistance": {
        "value": 5500000.0, "unit": "ohm"
      },
      "input_threshold": {
        "low": {
          "value": 0.42, "unit": "1"
        },
        "high": {
          "value": 0.52, "unit": "1"
        }
      },
      "reset_threshold": {
        "low": {
          "value": 0.42, "unit": "1"
        },
        "high": {
          "value": 0.44, "unit": "1"
        }
      }
    }
  ]
}
```

`packages/engine/tests/firmware/toggle.S` drives PB5 low at cycle 2, high at cycle 4, and toggles it every 100 cycles after that. A cycle of a 16 MHz crystal is 62,500 ps, so PB5 rises at 250,000 ps, falls at 6,500,000 ps, and rises again at 12,750,000 ps. Each edge lands at its picosecond, as [engine.md](engine.md#active-chips) says, and `#u1.logic.PB5` changes there.

`packages/engine/tests/circuits/atmega-blink` is this part with these values running `packages/engine/tests/firmware/blink.S`, which toggles PB5 at cycle 4 and every 8,000,000 cycles after, 0.5 s. PB5 drives the WP7113ID of `packages/engine/tests/circuits/led-5v` through 220 ohm to ground, so the LED blinks at 1 Hz with 12.5 mA on. Its 4 s run is four blinks, on a 10 ms grid.

`packages/engine/tests/circuits/atmega-button` is this part with these values running `packages/engine/tests/firmware/button.S`. A push button connects PD2 to ground. PD2's pull-up holds the pin high while the button is open, and the loop copies the inverse of PIND2 onto PB5, so the LED is on while the button is held. The contact bounces and the firmware does not debounce, so the LED follows the bounces a pass of the loop can sample. Seed 26 is the smallest seed that leaves one closed level shorter than a pass, and the LED does not follow that level. Its 1 s run is three presses of 200 ms, on a 10 ms grid.

`packages/engine/tests/circuits/atmega-analog-serial` is this part with these values running `packages/engine/tests/firmware/analog_serial.S`. A 10 kohm potentiometer sits across VCC and ground with its wiper on PC0, which is ADC0. Its position is 1023/2048, so the wiper is in the middle of ADC step 512. The firmware prints that code as decimal on TXD, one line per reading. The 19 ms run prints three lines on a 20 us grid, and its `traces/serial.csv` holds their bytes.

`packages/engine/tests/circuits/atmega-serial-echo` is this part with these values running `packages/engine/tests/firmware/serial_echo.S`. A `serial` source named `vrx` drives PD0 through 1 kohm. Its events send the byte `"1"` at 1 ms and the bytes `"0hi"` at 5 ms, both at 9600 baud. The firmware echoes each byte on TXD and drives PB5 from `'1'` and `'0'`. PB5 drives the WP7113ID of `packages/engine/tests/circuits/led-5v` through 220 ohm. The 12 ms run is on a 100 us grid, and its `traces/serial.csv` holds the four echoes.

`packages/engine/tests/circuits/atmega-fade` is this part with these values running `packages/engine/tests/firmware/fade.S`. PB1, which is OC1A, drives the WP7113ID of `packages/engine/tests/circuits/led-5v` through 220 ohm to ground. Timer1 is 8-bit phase-correct PWM with prescaler 64, about 490 Hz, and OC1A is non-inverting. `OCR1A` steps by 5 every 30 ms and reverses at 0 and 255. Its 1.57 s run climbs from off to full on and starts back down, on a 100 us grid.

`packages/engine/tests/circuits/atmega-i2c-scan` is this part with these values running `packages/engine/tests/firmware/sketch/i2c_scan.hex`, a Wire scanner. 4.7 kohm resistors pull PC4, SDA, and PC5, SCL, up to VCC, and nothing else is on the bus. The sketch probes each address from 1 to 126, none answers, and it prints `no devices` at 115200 baud. The 20 ms run is on a 100 us grid. Its `traces/i2c.csv` holds a START, an SLA+W with no acknowledge and a STOP for each address.

| Field | Unit | Rule |
| --- | --- | --- |
| `firmware` | Intel HEX text | records 00 to 05, an end record, every byte inside the 32 KB flash |
| `clock_hz` | `Hz`, a JSON integer | greater than 0 and at most 4294967295, else rule `positive` or `netlist.syntax` |
| `fuses.low`, `fuses.high`, `fuses.extended` | a JSON integer from 0 to 255 | the clock and mode rules below, else rule `unsupported` |
| `output.high_drop`, `output.low_drop` | `V` | at least 0, else rule `non_negative` |
| `output.high_resistance`, `output.low_resistance` | `ohm` | the resistor's |
| `pull_up_resistance`, `reset_pull_up_resistance` | `ohm` | the resistor's |
| `leakage_resistance` | `ohm` | the resistor's |
| `input_threshold.low`, `input_threshold.high` | `1` | inside (0, 1), else rule `fraction` |
| `reset_threshold.low`, `reset_threshold.high` | `1` | inside (0, 1), else rule `fraction` |
| `active_current` | `A` | optional, the resistor's |
| `reset_current` | `A` | optional, the resistor's |

A break is `netlist.value` with the dotted field, such as `input_threshold.high`, and a wrong unit is `netlist.unit` with the same field. A threshold's `low` must be at most its `high`. Above it is rule `above_high` on `low`.

`firmware` is the program as Intel HEX text, one record a line, in one JSON string. Parse reads it into flash from address 0. Record 00 places data, 01 ends the file, 02 and 04 move the base address, and 03 and 05 name a start address, which a part at reset does not use. The address field of record 01 is ignored the same way. A gap between records is erased flash, `0xFF`. Each byte is placed once. A line that is not a record, an end record with data, a bad checksum, any other record type, a record after the end, no end record, a byte at or past 32768, and a byte an earlier record placed are each `netlist.firmware` with the line and the rule. `packages/engine/scripts/firmware.sh` builds the repo's own programs.

`fuses` are the three bytes as programmed, so a programmed bit is 0 (Tables 28-6, 28-8 and 28-9). The model runs a crystal or ceramic resonator on `PB6` and `PB7`, `CKSEL` 0110 to 1111 in Table 9-1, and `clock_hz` is its frequency. A fuse setting the model does not run is `netlist.value` rule `unsupported` with the byte's field and its value:

- on `fuses.low`, any other `CKSEL`, because the model's only clock is the crystal at `clock_hz`. The factory low fuse, `0x62`, selects the internal RC oscillator and is `unsupported`.
- on `fuses.low`, a programmed `CKOUT`, because the model drives no clock on `PB0`.
- on `fuses.high`, a programmed `DWEN`, because the model has no debugWIRE on `RESET`.
- on `fuses.high`, a programmed `BOOTRST`, because the core always starts at word 0, and a real part would start in the boot section (Table 12-7).
- on `fuses.extended`, a reserved `BODLEVEL`, 000 to 011 in Table 29-12, which names no level.

`CKSEL0` with `SUT`, `CKDIV8`, `RSTDISBL`, `WDTON`, and `BODLEVEL` act as [avr.md](avr.md#fuses) says. The model reads nothing of `SPIEN`, `EESAVE`, or `BOOTSZ`.

The model is the core and its adapter in [avr.md](avr.md#behind-the-seam). Each port pin drives high through `output.high_resistance` from `VCC`, drives low through `output.low_resistance` into `GND_8`, pulls up through `pull_up_resistance` to `VCC`, or is off, as the firmware sets it. In each of those states it also leaks through `leakage_resistance` to `VCC` and through another to `GND_8`. It reads high above `input_threshold.high` times `V(VCC) - V(GND_8)` over `GND_8`, and low at or below `input_threshold.low` times it. `PC6` is `RESET` while `RSTDISBL` is unprogrammed. It then pulls up through `reset_pull_up_resistance`, leaks as an I/O pin does, and reads through `reset_threshold`, and a low level holds the chip in reset. The ADC samples `PC0` to `PC5`, `ADC6` and `ADC7` when the part wires them, `AVCC` and `AREF` against `GND_22`. With `active_current`, a conductance of `active_current / 5 V` joins `VCC` to `GND_8` while the core runs, so it draws `active_current` at a 5 V supply and scales with it, as the op-amp's `supply_current` does. With `reset_current`, the conductance is `reset_current / 5 V` instead while a reset, from `RESET` low or a brown-out, holds the chip. The catalog's ATmega328P takes 2.0 mA there, DS40002061B Figure 31-377 on the 5.0 V curve at 16 MHz, against its 11.4 mA running. Without `reset_current` the chip draws `active_current` in reset too, and without either the core draws nothing, so a netlist written before the fields keeps its meaning. The conductance joins no new DC path, since every port pin's leakage already joins `VCC` to `GND_8`.

The example's values but one are typical curves at VCC = 5 V, read by eye, not digitized. `leakage_resistance` is derived from a bound. All are the documented defaults to copy.

| Field | Value | Reading |
| --- | --- | --- |
| `output.*_resistance` | 25 ohm | Figures 31-353 and 31-355 at 25 C: about 0.45 V at 20 mA sunk and 0.55 V under VCC at 20 mA sourced, 22.5 and 27.5 ohm. Both sides take 25 ohm, between the two, which also matches a bench fit, so an eye reading is not frozen as a measured difference. |
| `output.*_drop` | 0 V | Both curves start at their rail at 0 mA. |
| `pull_up_resistance` | 35,714 ohm | Figure 31-348, about 140 uA at 0 V, straight to 0 at 5 V. |
| `reset_pull_up_resistance` | 46,296 ohm | Figure 31-351, about 108 uA at 0 V. |
| `input_threshold` | 0.42 and 0.52 | Figures 31-357 and 31-356, about 2.1 V falling and 2.6 V rising. |
| `reset_threshold` | 0.42 and 0.44 | Figures 31-360 and 31-359, about 2.1 V falling and 2.2 V rising. |
| `leakage_resistance` | 5.5 Mohm | Derived from Table 30-1, which bounds `IIL` and `IIH` at 1 uA at VCC = 5.5 V. With the pin at one rail, 5.5 V across 5.5 Mohm to the other is that 1 uA. It is an approximation: the leakage at its bound, written as a resistance. Table 30-1 gives `RESET` no bound of its own, and `PC6` takes the I/O pin's. |

Table 30-1's limits are wider. The pull-up there is 20 to 50 kohm and the reset pull-up 30 to 60 kohm. `VIL` is at most 0.3 VCC and `VIH` at least 0.6 VCC, where a read is ensured, not where it trips, and `RESET`'s are at most 0.1 VCC and at least 0.9 VCC. A corner run takes one of those limits instead.

Each port pin, `PC6` included, is a DC path to `VCC` and to `GND_8` in every drive state, through its leakage, and the solve stamps that leakage in every state. An output stage beside it moves the pin by about 5 ppm of the supply. A net that only an off pin holds solves on that leakage, not on gmin. `PB6`, `PB7`, `AREF`, `ADC6` and `ADC7` join nothing, so a net reached only through them is `net.floating`. So is a net reached only through `AVCC`. §1.1.7 makes `AVCC` the supply of the ADC and of `PC3` to `PC0`, and says it must be connected to `VCC` even when the ADC is unused. The model draws every pin's stages, `PC3` to `PC0` included, from `VCC`, and reads `AVCC` only at an ADC sample, so it joins nothing. That is the datasheet's circuit while `AVCC` is wired to `VCC`, and an `AVCC` left on a net of its own is reported. An unused port pin on a net of its own is not. An off pin with no load sits at half the supply, inside the band between the defaults' `input_threshold.low` and `input_threshold.high`, so it reads low from power-on and then keeps its last level. Thresholds that put half the supply outside the band read it as that side.

`ratings` is optional, and a part without it is unrated. Its five keys are required, and each is a value object or `null`, as [Ratings](#ratings) says. `pin_current` bounds each port pin's current, and `supply_current` the current through `VCC` and through `GND_8`, both in `A`. `rail_margin` is how far, in `V`, a pin may go below `GND_8`, and every pin but `VCC` and `RESET` above `VCC`. `reset_voltage` is the most `PC6` may sit above `GND_8` while it is `RESET`, and `supply_voltage` the most `VCC` may. The documented values are DS40002061B §29.1's absolute maxima, page 308:

| Key | Value | §29.1 |
| --- | --- | --- |
| `pin_current` | 0.04 A | DC Current per I/O Pin, 40.0 mA |
| `supply_current` | 0.2 A | DC Current VCC and GND Pins, 200.0 mA |
| `rail_margin` | 0.5 V | Voltage on any Pin except RESET with respect to Ground, -0.5 V to VCC + 0.5 V |
| `reset_voltage` | 13.0 V | Voltage on RESET with respect to Ground, -0.5 V to +13.0 V |
| `supply_voltage` | 6.0 V | Maximum Operating Voltage, 6.0 V |

```json
"ratings": {
  "pin_current": { "value": 0.04, "unit": "A" },
  "supply_current": { "value": 0.2, "unit": "A" },
  "rail_margin": { "value": 0.5, "unit": "V" },
  "reset_voltage": { "value": 13.0, "unit": "V" },
  "supply_voltage": { "value": 6.0, "unit": "V" }
}
```

The run works out each port pin's current from the row and the drive the firmware has in force: the output stage, the pull-up and both leakages above, out of the pin into the circuit. `VCC`'s current is the sum every port pin draws from `VCC`, and `GND_8`'s the sum every port pin sinks into it. With `active_current` or `reset_current`, both sums also hold the core's own draw through the conductance in force for that row. [diagnostics.md](diagnostics.md#ratingcurrent) says how the run reports a current over its rating, and [diagnostics.md](diagnostics.md#ratingvoltage) a pin outside its window. These are the model's numbers. The stage is linear, so a pin shorted to the other rail reads 200 mA at 5 V, about twice a real driver's. No clamp diode conducts, so a pin past a rail draws no clamp current.

An ATmega328P has no `i`, `v`, or `p`. Its logic signals are its watched pins, described in [Refs](#refs).

`packages/engine/tests/circuits/atmega-board-blink` places this part on the development board fragment and runs `packages/engine/tests/firmware/sketch/blink.hex`. `packages/engine/tests/circuits/atmega-board-button` is that fragment running `blocks/nano-button-led/build/button.ino.hex`, with a switch from `d2` to `v5` and a 10 kohm pull-down to `gnd`. `packages/engine/tests/circuits/atmega-board-knob` is that fragment running `packages/engine/tests/firmware/sketch/knob.hex`, with a knob's two contacts on `d2` and `d3`, and the sketch prints a count for each.

### SPI replay

`kind` is `spi_replay`, an SPI host that replays a list of transactions. It is an active chip with no clock of its own, and its records are the transactions, as [engine.md](engine.md#records) says. Pins are `VCC`, `GND`, `CS`, `SCLK` and `D0` to `D3`. Every pin is required.

`packages/engine/tests/circuits/spi-replay` replays a display panel's start-up on a 3.3 V supply at 40 MHz. Each register write is the opcode 0x02 and a 24-bit address that holds the command in its middle byte, all on one lane, then the parameters on one lane. The pixel write is the opcode 0x32 and the address 0x002C00 on one lane, then two bytes on four lanes with CS held, then two more bytes alone, which continue it. The part below shows that pixel write's two events, and the example's other five are the register writes around it:

```json
{
  "name": "rp1",
  "kind": "spi_replay",
  "pins": {
    "VCC": "v3v3", "GND": "gnd", "CS": "cs", "SCLK": "sclk",
    "D0": "d0", "D1": "d1", "D2": "d2", "D3": "d3"
  },
  "clock_hz": { "value": 40000000, "unit": "Hz" },
  "output": {
    "high_drop": { "value": 0.0, "unit": "V" },
    "high_resistance": { "value": 25.0, "unit": "ohm" },
    "low_drop": { "value": 0.0, "unit": "V" },
    "low_resistance": { "value": 25.0, "unit": "ohm" }
  },
  "events": [
    {
      "t_ps": 50000000,
      "phases": [{ "bits": 8, "lanes": 1, "value": 50 }, { "bits": 24, "lanes": 1, "value": 11264 }],
      "lanes": 4, "bytes": [248, 0], "cs": "held"
    },
    { "t_ps": 52000000, "phases": [], "lanes": 4, "bytes": [7, 224], "cs": "released" }
  ]
}
```

The model is the bus's idle levels and the list.

- `CS` drives high and `SCLK` and `D0` to `D3` drive low for the whole run, SPI mode 0's idle levels. High is `output.high_drop` under `VCC` behind `output.high_resistance`, and low is `output.low_drop` over `GND` behind `output.low_resistance`. The drops are sources, so source stepping scales them.
- Each event is one transaction from `t_ps`. Its `phases` run first, in order, each `bits` bits of `value`, most significant first, on `lanes` lanes. Then `bytes` run on `lanes` lanes. A phase takes `ceil(bits / lanes)` SCLK cycles, and the bytes take `ceil(8 * count / lanes)`. The event ends at `t_ps + floor(cycles * 10^12 / clock_hz)` picoseconds, computed in 128-bit arithmetic.
- `cs` is `held` or `released`, the chip select at the event's end. A transaction with no phases after one that ends `held` continues it.
- The event is sent as a `Spi` record at its end, with the nets of the part's `CS`, `SCLK` and `D0` to `D3` pins. No trace that reads a net shows the transfer. `traces/link.csv` holds it, as [run-folder.md](run-folder.md#traces) says.

| Field | Unit | Rule |
| --- | --- | --- |
| `clock_hz` | `Hz`, a JSON integer | greater than 0 and at most 4294967295, else rule `positive` or `netlist.syntax` |
| `output.high_resistance`, `output.low_resistance` | `ohm` | the resistor's |
| `output.high_drop`, `output.low_drop` | `V` | at least 0, else rule `non_negative` |
| `events[k].phases[j].bits` | count | an integer from 1 to 32 |
| `events[k].phases[j].value` | count | an integer below `2^bits` |
| `events[k].phases[j].lanes`, `events[k].lanes` | count | 1, 2 or 4 |
| `events[k].bytes` | count | integers from 0 to 255 |

A `bits`, `value` or `lanes` outside its rule, a byte above 255, and a `cs` other than `held` or `released` are `netlist.syntax`. An event that breaks a schedule rule is `netlist.schedule` on field `events`, with the `serial` waveform's rules: `at_zero`, `not_increasing`, `overlap` and `overflow`, and `same_state` for an event with no phases and no bytes, which sends nothing. An event that starts exactly when the previous one ends is legal.

A `spi_replay` has no `ratings` object and no `i`, `v`, or `p`. It has no logic signals. It joins `CS`, `SCLK` and `D0` to `D3` each to `VCC` and to `GND`, through the stage its drive stamps.

### ST77916

`kind` is `st77916`, the controller of a round 360 by 360 TFT panel on a QSPI bus. It is an active chip with no clock of its own. It takes the `Spi` records whose chip select and clock are on its `CS` and `SCLK` nets and the `Duty` records on its `BLK` net, as [engine.md](engine.md#records) says. Pins are `VCC`, `GND`, `CS`, `SCLK`, `D0` to `D3`, `RST`, `TE` and `BLK`. Every pin is required.

`packages/engine/tests/circuits/st77916-replay` puts it on a `spi_replay` at 80 MHz and 3.3 V. The replay sends a short start-up, then closes seven pixel writes, each a pattern with more than one value. The start-up is `SWRESET`, `SLPOUT`, `COLMOD` 0x55, `MADCTL`, `CASET`, `RASET` and `DISPON`, then four writes to registers the model stores and does not run. One write overfills its window and wraps. One is a `RAMWRC` that continues a `RAMWR`. `packages/engine/scripts/st77916_example.py` writes the example and its replay list, `events.json`. The panel part is:

```json
{
  "name": "lcd1",
  "kind": "st77916",
  "pins": {
    "VCC": "v3v3", "GND": "gnd", "CS": "cs", "SCLK": "sclk",
    "D0": "d0", "D1": "d1", "D2": "d2", "D3": "d3",
    "RST": "rst", "TE": "te", "BLK": "v3v3"
  },
  "output": {
    "high_drop": { "value": 0.0, "unit": "V" },
    "high_resistance": { "value": 25.0, "unit": "ohm" },
    "low_drop": { "value": 0.0, "unit": "V" },
    "low_resistance": { "value": 25.0, "unit": "ohm" }
  },
  "input_threshold": {
    "low": { "value": 0.3, "unit": "1" },
    "high": { "value": 0.7, "unit": "1" }
  }
}
```

The model is the panel's command set and its frame memory.

- A command is one 32-bit word, `opcode << 24 | command << 8`, the first 32 bits `CS` carries: a transaction's phases, then its bytes, on through each transaction that ends with `CS` held until the word is whole. Bits 16 to 23 are ignored, and so is the page byte, the low eight bits. Opcode 0x02 is the register opcode. Opcode 0x32 is the pixel opcode. `RAMWR` and `RAMWRC` take the pixel opcode. Every other command takes the register opcode. The SPI replay of this example sends the word as an 8-bit command phase and a 24-bit address phase, and ESP-IDF's `esp_lcd` sends it as four bytes of data on one lane. The model checks the opcode of every command. It checks the data lanes only on the bytes after the word, because a transaction with no such bytes clocks no data. The word's own bits go on one lane, register data uses one lane, and pixel data uses four. A wrong opcode, or bytes after the word on the wrong lanes, drops the command and its data until `CS` is released, rule `command`. Phases past the word's 32 bits do the same, and so do a bit of the word on more than one lane, a phase or bytes, and a release of `CS` before the word is whole. A continuation whose data lanes do not match drops that transaction's data, and a release still closes the open command. A transaction with phases while a command is open is dropped, rule `command`, and a release still closes the open command.
- A transaction that ends with `CS` held leaves its command open, and the next transaction's bytes continue it. A command closes when `CS` is released, or when `RST` goes low.
- A register command runs when it closes. `SWRESET` (0x01) resets the registers. `SLPIN` and `SLPOUT` (0x10, 0x11) set the sleep flag, `INVOFF` and `INVON` (0x20, 0x21) the inverted flag, and `DISPOFF` and `DISPON` (0x28, 0x29) the display flag. `CASET` and `RASET` (0x2A, 0x2B) take a big-endian start and end, both ends included, and an end below 360 at or after the start. `MADCTL` (0x36) and `COLMOD` (0x3A) take one byte. A `CASET`, `RASET`, `MADCTL` or `COLMOD` short of bytes or out of range changes nothing, rule `command`. Every other register command replaces the stored bytes of that command and is never run. The panel keeps the last bytes of each such command, at most 256 commands. `registers` in the panel state is how many of those commands it holds.
- `RAMWR` (0x2C) writes from the window's first column and row, and `RAMWRC` (0x3C) goes on from the last pixel. Each two bytes are one pixel, kept in the frame memory in the order they arrived. Firmware that sends the high byte first, as this example does and as `COLMOD` 0x55 on this panel expects, gives big-endian RGB565. A reader must not assume little-endian. A pixel goes along its row, to the next row at the window's last column, and back to the first row after the last one. `MADCTL` maps each pixel into the frame memory. Bit 0x40 mirrors the column, bit 0x80 mirrors the row, and then bit 0x20 exchanges them. That order is this model's choice until a capture from a real panel checks it. The other bits stay in the byte and move no pixel. A pixel write needs `COLMOD`'s low three bits to be 5, 16 bits a pixel. Otherwise it is dropped with its data, rule `pixel_format`.
- A pixel write that took bytes is one row of `traces/display.csv` when it closes, as [run-folder.md](run-folder.md#traces) says. One that took none is no row.
- A reset sets the sleep flag, clears the display and inverted flags, sets `MADCTL` to 0, `COLMOD` to 0x66, the window to the whole panel, and drops every stored register. The frame memory keeps its pixels, and is all zero bytes at power-on.
- `RST` reads high above `input_threshold.high` times `V(VCC) - V(GND)` over `GND`, and low at or below `input_threshold.low` times it. It is a watch, described in [Chip events](engine.md#chip-events). Going low resets the panel and drops an open command with no row. While it is low the panel drops every transaction.
- `TE` drives low for the whole run, `output.low_drop` over `GND` behind `output.low_resistance`. The panel never signals a tearing effect, so firmware that waits on `TE` stalls.
- `BLK` keeps the last `Duty` record it took as `brightness`. It changes no pixel.
- The panel waits for nothing. The 120 ms a real panel needs after `SWRESET` and `SLPOUT` is not kept, and sleep and the display flag change no pixel of the frame memory.
- The glass is round. Pixel `(x, y)` is on it when its centre, `(x + 0.5, y + 0.5)`, lies inside the circle of radius 180 about `(180, 180)`, the corner pixels 179 and 180 share on both axes. In doubled integers that is `(2x + 1 - 360)² + (2y + 1 - 360)² <= 360²`. No pixel's centre lies on the circle, because two odd squares never sum to 360², so the edge rule never decides a pixel. Row 0 holds columns 167 to 192, and 101780 pixels are on the glass.
- `INVON` shows the frame memory as written, and `INVOFF` shows each channel inverted. That is a board observation, not a datasheet reading. With the vendor start-up, which ends in `INVON`, a frame of red, green, blue, white and black bands written high byte first showed red, green, blue, white and black. The `inverted` flag after a reset is false, so firmware that skips `INVON` sees inverted colours on the glass.
- What the glass shows is drawn from the flags and the frame memory: transparent off the circle, opaque black while the display flag is clear, the panel sleeps, or `RST` is low, and otherwise each pixel as big-endian RGB565 under the `INVON` sense above. [run-folder.md](run-folder.md#what-the-glass-shows) gives every rule. `BLK` dims nothing there. A viewer applies `brightness`.

A dropped transaction is the warning `chip.protocol`, once per rule at its first time. A transaction whose data lanes are not on the panel's own `D0` to `D3` is dropped as the warning `chip.link_miswired`, once at its first time. [diagnostics.md](diagnostics.md) has both.

| Field | Unit | Rule |
| --- | --- | --- |
| `output.high_resistance`, `output.low_resistance` | `ohm` | the resistor's |
| `output.high_drop`, `output.low_drop` | `V` | at least 0, else rule `non_negative` |
| `input_threshold.low`, `input_threshold.high` | `1` | inside (0, 1), else rule `fraction` |

A break is `netlist.value` with the dotted field, and a wrong unit is `netlist.unit` with the same field. `input_threshold.low` must be at most `input_threshold.high`. Above it is rule `above_high` on `low`.

An `st77916` has no `ratings` object and no `i`, `v`, or `p`. Its logic signal is `RST`. It joins `TE` to `VCC` and to `GND` through the stage its drive stamps. `CS`, `SCLK`, `D0` to `D3`, `RST` and `BLK` draw no current, so each needs another DC path.

### CST816

`kind` is `cst816`, a touch controller that is an I2C target at address 0x15. It is an active chip with no clock of its own. Pins are `VCC`, `GND`, `SCL`, `SDA`, `INT` and `RST`. Every pin is required. `SDA` and `INT` are open drain into `GND`, `low_drop` over `GND` behind `low_resistance`. `SCL` is an input. The part does not hold `SCL`, so a target that clock-stretches is not modelled, as [engine.md](engine.md#i2c-targets) says. The model answers while idle, and it answers on the first transfer after `RST` rises. Auto-sleep and the start-up time after reset are not modelled.

`packages/engine/tests/circuits/atmega-cst816` puts one on an ATmega328P at 3.3 V and 8 MHz. 2.2 kohm resistors pull `SDA` and `SCL` up to `VCC`, and the bus has no capacitance. 10 kohm pulls `INT` up. `RST` is tied to `VCC`. The part runs `packages/engine/tests/firmware/sketch/cst816.hex`, which reads register 0x02 for five bytes and prints the finger count, X and Y at 9600 baud, twice, 20 ms apart. The part is:

```json
{
  "name": "u2",
  "kind": "cst816",
  "pins": {
    "VCC": "vcc", "GND": "gnd", "SCL": "pc5", "SDA": "pc4", "INT": "pd2", "RST": "vcc"
  },
  "low_drop": { "value": 0.0, "unit": "V" },
  "low_resistance": { "value": 133.3, "unit": "ohm" },
  "input_threshold": {
    "low": { "value": 0.3, "unit": "1" },
    "high": { "value": 0.7, "unit": "1" }
  },
  "touches": [
    { "t_ps": 20000000000, "x": 291, "y": 69, "down": true },
    { "t_ps": 41000000000, "x": 16, "y": 32, "down": true }
  ]
}
```

`low_resistance` of 133.3 ohm is 0.4 V at 3 mA, a stated assumption for the open-drain low. `input_threshold` of 0.3 and 0.7 is a guess.

Registers `0x01` to `0x06` and `0xA7` are the layout the open drivers read. No datasheet was in hand. The chip id byte below is the same kind of guess.

- `0x01` is the gesture. It stays 0. The model has no gesture classifier.
- `0x02` is the finger count, 1 while the finger is down and 0 while it is up.
- `0x03` and `0x04` are X high and X low. `0x05` and `0x06` are Y high and Y low. Each coordinate is 12 bits, from 0 to 4095. The high register holds bits 11 to 8 in bits 3 to 0, and bits 7 to 4 stay 0. Event flags in those bits are not modelled.
- `0xA7` is the chip id, `0xB4`. That byte is a guess: it is the byte those drivers expect for a CST816S, and no sheet was in hand.
- Every other register reads 0 until a host writes it.

The first data byte of a write sets the register pointer. Each later write byte stores at the pointer and then advances it by one, wrapping at 256. Each read byte returns the pointer and then advances it the same way. The advance is inferred from the burst those drivers read. A touch writes `0x01` to `0x06` and leaves the pointer. A finger up keeps the last X and Y and sets the finger count to 0. A host write can overwrite any of those bytes. `RST` low restores the chip id and clears the rest on that edge, and drops a touch that arrives while it is held. `INT` releases 1 ps later. `SDA` releases 300 ns later, whatever `SCL` reads. `INT` is released at rest. Each touch drives it low for 100 us, starting 1 ps after the touch. That width is a guess. A second touch during the pulse extends the one release.

`touches` may be absent. Each event is one point, after t = 0 and at least 2000 ps after the previous event. `x` and `y` are integers from 0 to 4095. A point equal to the previous one is still an event. A session appends to this array the way it appends a serial send, as [engine.md](engine.md#sessions) says.

| Field | Unit | Rule |
| --- | --- | --- |
| `low_resistance` | `ohm` | the resistor's |
| `low_drop` | `V` | at least 0, else rule `non_negative` |
| `input_threshold.low`, `input_threshold.high` | `1` | inside (0, 1), else rule `fraction` |
| `touches[].x`, `touches[].y` | | from 0 to 4095, else rule `span` |
| `touches[].t_ps` | `ps` | the schedule rules, on field `touches` |

A break is `netlist.value` with the dotted field, and a wrong unit is `netlist.unit` with the same field. `input_threshold.low` must be at most `input_threshold.high`. Above it is rule `above_high` on `low`. A touch at t = 0, a touch that is not after the previous one, and a touch less than 2000 ps after it are `netlist.schedule`, as [diagnostics.md](diagnostics.md#netlistschedule) says.

A `cst816` has no `ratings` object and no `i`, `v`, or `p`. Its logic signals are `SCL`, `SDA`, `INT` and `RST`. It joins `SDA` to `GND` and `INT` to `GND` through the stage its drive stamps. `SCL`, `RST` and `VCC` draw no current, so each needs another DC path.

### WS2812

`kind` is `ws2812`, a chain of addressable LEDs on one data pin. It is an active chip with no clock of its own. It watches `DIN` and takes the `Pulses` records on that net, as [engine.md](engine.md#records) says. Pins are `VCC`, `GND` and `DIN`, each required, and `DOUT`, which a part wires exactly when it gives `dout`. Each LED takes the next 24 bits. Without `dout` the bits past the last LED are discarded, and with it they go on through `DOUT`. Green is the first byte, then red, then blue, most significant bit first. A low longer than the reset latches the colours into `observations.json` and `traces/leds.csv`.

`leds` may be absent. It is then 13. An integer from 0 to 65535 outside 1 to 1024 is `netlist.value` with rule `leds`, and its message prints the count as an integer. Any other number, a negative, a fraction, `13.0`, or one above 65535, is `netlist.syntax`.

The bit times are the Worldsemi WS2812E-1313 datasheet's, V1.1, page 3. A high of 220 ns to 380 ns, T0H, is a 0, and a high of 580 ns to 1 µs, T1H, is a 1, each end included. A low up to 280 µs ends the bit, however short or long. The same page gives T0L and T1L as 580 ns to 1 µs and a bit of at least 1.25 µs, and the WS2812E 5050 datasheet, V1.0, page 3, gives T1L as 220 ns to 420 ns. The chain checks no low against either, so a train timed to either sheet decodes, the 300 ns high and 900 ns low of an RMT 0 and its 900 ns high and 300 ns low 1 among them. RES is a low longer than 280 µs, and it latches. The table gives ">280µs", so a low of exactly 280 µs is no reset and the chain latches at the fall plus 280 µs plus 1 ps. A high outside both windows drops that bit, and the first drop is `chip.protocol` rule `bit`.

With `dout`, once every LED holds its 24 bits, each high on `DIN` goes on to `DOUT` `dout.delay` later, its rise and its fall alike, until the chain latches. `DOUT` is low before that and after the latch. It drives high through `dout.output.high_drop` under `VCC` behind `dout.output.high_resistance`, and low through `dout.output.low_drop` over `GND` behind `dout.output.low_resistance`, as an NE555's OUT does. The SK6812MINI-HS datasheet, Rev. 04, section 11, gives the delay from `DIN` to `DOUT` as at most 500 ns, rise and fall alike, and gives no output widths, so `DOUT` passes each high as long as it came. A second chain whose `DIN` is that net takes those bits as it would from a chip, so parts chained one after the other each take their own 24 bits. A high that a `Pulses` record carried has already ended when the record lands, so it does not go on to `DOUT`. The first is `chip.protocol` rule `dout_record`, and `DOUT` stays low. At most 8 `DOUT` edges are due at once. The datasheet's times queue 2, and a line that changes faster drops the newest high whole.

The same sheet, page 2, gives `VIH` as at least 0.7 VDD and `VIL` as at most 0.7 V, both at VDD 5 V. `VIL` is a voltage there, not a fraction of the supply. The example's `input_threshold.low` is that 0.7 V over 5 V, 0.14 of the supply, so it holds at 5 V.

`packages/engine/tests/circuits/atmega-ws2812` puts 13 LEDs on an ATmega328P at 5 V and 16 MHz. `PB0` drives `DIN` through 100 ohm, and 10 kohm pulls `DIN` down, so the pin is low until the sketch raises it. The sketch is `packages/engine/tests/firmware/sketch/ws2812.hex`. A 0 is high for 5 cycles and low for 15, and a 1 is high for 10 and low for 10. LED `i` is green `i`, red `i + 16` and blue `i + 32`. After the frame the pin stays low for 300 µs, which is the reset. `packages/engine/tests/circuits/ws2812-replay` sends that same frame as one `Pulses` record. The part is:

```json
{
  "name": "ring",
  "kind": "ws2812",
  "pins": { "VCC": "vcc", "GND": "gnd", "DIN": "din" },
  "leds": 13,
  "input_threshold": {
    "low": {
      "value": 0.14, "unit": "1"
    },
    "high": {
      "value": 0.7, "unit": "1"
    }
  }
}
```

| Field | Unit | Rule |
| --- | --- | --- |
| `leds` | count | an integer from 1 to 1024. 0 to 65535 outside that is rule `leds`, and any other number is `netlist.syntax`. Absent means 13 |
| `input_threshold.low`, `input_threshold.high` | `1` | inside (0, 1), else rule `fraction` |
| `dout.output.high_drop`, `dout.output.low_drop` | `V` | at least 0, else rule `non_negative` |
| `dout.output.high_resistance`, `dout.output.low_resistance` | `ohm` | the resistor's |
| `dout.delay` | `ps` | an integer from 1 to 1000000. 0 is rule `positive`, and above 1000000, T1H's longest high, is rule `delay` |

`dout` may be absent, and the chain then has no `DOUT`. `dout` without a `DOUT` pin, or a `DOUT` pin without `dout`, is `netlist.pins`. A break is `netlist.value` with the dotted field, and a wrong unit is `netlist.unit` with the same field. `input_threshold.low` must be at most `input_threshold.high`. Above it is rule `above_high` on `low`.

A `ws2812` has no `ratings` object and no `i`, `v`, or `p`. Its logic signal is `DIN`. It joins `DOUT` to `VCC` and to `GND` through the stage its drive stamps, and no other pins, so `VCC`, `GND` and `DIN` each need another DC path.

### DHT22

`kind` is `dht22`, a DHT22 temperature and humidity sensor, also sold as the AM2302. It is an active chip with no clock of its own. Pins are `VCC`, `DATA` and `GND`. Every pin is required. `DATA` is open drain into `GND` behind `low_resistance`, and the part reads `DATA` through `input_threshold`, fractions of its supply. It reads the netlist's `temperature` and `humidity`, which the netlist's `world` must declare, as [World](#world) says. The sensor's internal pull-up is not part of this kind. A part file adds it as a resistor from `VCC` to `DATA`.

Every time is the typical of Aosong's AM2302 product manual, version 1.1, table 6. A host low on `DATA` of at least 0.8 ms (`Tbe`) is a start. 30 us after the line rises (`Tgo`), the sensor pulls `DATA` low for 80 us and releases it for 80 us (`Trel`, `Treh`). Then each of 40 bits, most significant first, is low for 50 us (`TLOW`) and released for 26 us for a 0 or 70 us for a 1 (`TH0`, `TH1`). Last, the sensor pulls the line low for 50 us (`Ten`) and releases it. The bits are humidity in tenths of a %RH, then temperature in tenths of a degC, 16 bits each, then the low byte of the sum of those four bytes (table 5). Temperature's top bit is its sign and the 15 below it are the magnitude, so -10.1 degC is `0x8065`. A value rounds to the nearest tenth, and one outside 0 to 100 %RH or -40 to 80 degC reads as its nearer end.

A read sends the last measurement, and the end of each reply starts the next one (the note under figure 6, and section 7.3). So a read sends what the sensor measured as the read before it ended. The first read sends what it measured at 2 s, the end of its warm-up (section 7.4). A measurement takes no time, and it reads the world at that time. The sensor answers no start inside the 2 s warm-up, or within 2 s of the last start it answered (section 7.1, note 3). Such a start gets no reply and is `chip.protocol` rule `interval` the first time. A low under 0.8 ms is no start and breaks no rule.

`packages/engine/tests/circuits/atmega-dht22` is a Nano with a DHT22 on `D2`, the part's 45 kohm pull-up and 5.1 kohm more. `AREF` sits on `5V`, so no net floats. It runs `packages/engine/tests/firmware/sketch/dht22.hex`, which reads the sensor every 2 s and prints `n=1 t=23.4 h=61.7`. The world steps from 23.4 degC and 61.7 %RH to -10.1 degC and 40 %RH at 3 s, and the third read prints `n=3 t=-10.1 h=40.0`. The part is:

```json
{
  "name": "dht__sensor",
  "kind": "dht22",
  "pins": { "VCC": "nano__5V", "DATA": "nano__D2", "GND": "nano__GND" },
  "low_resistance": {
    "value": 37.5, "unit": "ohm"
  },
  "input_threshold": {
    "low": {
      "value": 0.3, "unit": "1"
    },
    "high": {
      "value": 0.7, "unit": "1"
    }
  }
}
```

| Field | Unit | Rule |
| --- | --- | --- |
| `low_resistance` | `ohm` | the resistor's |
| `input_threshold.low`, `input_threshold.high` | `1` | inside (0, 1), else rule `fraction` |

A break is `netlist.value` with the dotted field, and a wrong unit is `netlist.unit` with the same field. `input_threshold.low` must be at most `input_threshold.high`. Above it is rule `above_high` on `low`.

A `dht22` has no `ratings` object and no `i`, `v`, or `p`. Its logic signal is `DATA`. It joins `DATA` to `GND` through the stage its drive stamps. `VCC` draws no current, so it needs another DC path.

### Pulses replay

`kind` is `pulses_replay`, a pin that replays a list of timed levels. It is an active chip with no clock of its own, and its records are the trains, as [engine.md](engine.md#records) says. Pins are `VCC`, `GND` and `OUT`. Every pin is required.

`OUT` drives `idle` for the whole run. High is `output.high_drop` under `VCC` behind `output.high_resistance`, and low is `output.low_drop` over `GND` behind `output.low_resistance`. The drops are sources, so source stepping scales them. The bits are in the record. No trace that reads the pin shows them.

Each event is one train from `t_ps`. `pulses` is a list of `{ "level", "ps" }`. The levels run in order, each for `ps` picoseconds, and the event ends at `t_ps` plus those durations. The event is sent as a `Pulses` record at its end, on the part's `OUT` net, with `idle` as the record's idle level. An event that starts exactly when the previous one ends is legal.

| Field | Unit | Rule |
| --- | --- | --- |
| `output.high_resistance`, `output.low_resistance` | `ohm` | the resistor's |
| `output.high_drop`, `output.low_drop` | `V` | at least 0, else rule `non_negative` |
| `events[k].pulses[j].ps` | `ps` | greater than 0, else rule `positive` |

An event that breaks a schedule rule is `netlist.schedule` on field `events`, with the `serial` waveform's rules: `at_zero`, `not_increasing`, `overlap` and `overflow`, and `same_state` for an event with no pulses. A duration that would end past the largest time is `overflow`.

A `pulses_replay` has no `ratings` object and no `i`, `v`, or `p`. It has no logic signals. It joins `OUT` to `VCC` and to `GND` through the stage its drive stamps.

### ESP32-S3

`kind` is `esp32s3`, an ESP32-S3 running a bare-metal program or booting an app from its flash, as [esp32s3.md](esp32s3.md) describes. Pins are `VCC`, `GND`, and the GPIOs a board can wire: `GPIO0` to `GPIO21`, `GPIO26` to `GPIO32`, and `GPIO38` to `GPIO48`. GPIO22 to GPIO25 do not exist, and GPIO33 to GPIO37 belong to the in-package PSRAM. `VCC` and `GND` are required. Every GPIO is optional. A GPIO with no net is neither stamped nor watched, and its pad reads what it drives. A pin outside that list is `netlist.pins`.

`packages/engine/tests/circuits/esp32s3-blink` drives the WP7113ID of `packages/engine/tests/circuits/led-5v` from GPIO21 through 330 ohm at 3.3 V. Its part is:

```json
{
  "name": "u1",
  "kind": "esp32s3",
  "pins": { "VCC": "vcc", "GND": "gnd", "GPIO21": "gpio21" },
  "firmware": {
    "elf": {
      "sha256": "36f33508452c63a7527edff29db0da3e534714c5737c660a441a1c056effe3aa",
      "bytes": 3800
    }
  },
  "output": {
    "high_drop": { "value": 0.0, "unit": "V" },
    "high_resistance": { "value": 16.5, "unit": "ohm" },
    "low_drop": { "value": 0.0, "unit": "V" },
    "low_resistance": { "value": 17.68, "unit": "ohm" }
  },
  "pull_up_resistance": { "value": 45000.0, "unit": "ohm" },
  "leakage_resistance": { "value": 66000000.0, "unit": "ohm" },
  "input_threshold": {
    "low": { "value": 0.25, "unit": "1" },
    "high": { "value": 0.75, "unit": "1" }
  }
}
```

`packages/engine/tests/circuits/esp32s3-hello` boots an ESP-IDF app from the flash instead. Its part replaces `firmware` with the flash's regions and names the mask ROM beside them:

```json
"rom": {
  "sha256": "c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd",
  "bytes": 949552
},
"firmware": {
  "flash": [
    {
      "offset": 32768,
      "sha256": "978d83c84af3a8b59ecf9763499ac8c18f4d4231f9499bbe4b2acd99863325be",
      "bytes": 3072
    },
    {
      "offset": 65536,
      "sha256": "98f9055cacf28d25297beccd746ff861abfebb49da89518eb87867f5954a03b9",
      "bytes": 151664
    }
  ]
}
```

`firmware` holds exactly one of `elf` and `flash`. `rom` is present exactly when `firmware` is `flash`.

| Field | Unit | Rule |
| --- | --- | --- |
| `firmware.elf` | a blob | the [Blobs](#blobs) rules, then an ELF executable for the chip's SRAM |
| `firmware.flash` | blobs, each at an `offset` | the [Blobs](#blobs) rules for each, then a flash with an app the bootloader would start |
| `rom` | a blob | only beside `firmware.flash`, the revision 0 ROM, then the [Blobs](#blobs) rules |
| `console` | events of `t_ps` and `bytes` | each after t = 0 and after the previous one, with a byte, else `netlist.schedule` |
| `output.high_drop`, `output.low_drop` | `V` | at least 0, else rule `non_negative` |
| `output.high_resistance`, `output.low_resistance` | `ohm` | the resistor's |
| `pull_up_resistance` | `ohm` | the resistor's |
| `pull_down_resistance` | `ohm` | optional, the resistor's |
| `leakage_resistance` | `ohm` | the resistor's |
| `input_threshold.low`, `input_threshold.high` | `1` | inside (0, 1), else rule `fraction` |
| `efuse` | a name | optional, `synthetic` when absent, one of the sets in `packages/socs/espressif/src/s3/efuse.txt`, else `netlist.syntax` |

A break is `netlist.value` with the dotted field, and a wrong unit is `netlist.unit` with the same field. `input_threshold.low` must be at most `input_threshold.high`. Above it is rule `above_high` on `low`. `pull_down_resistance` may be absent. A store that sets `FUN_WPD` on a pad with a net, while the part names none, ends the run with `chip.unsupported_setting`, `IO_MUX` `pull_down`, and writes nothing. A pad with no net is never stamped, so that store is kept. A netlist that never turns a netted pad's pull-down on keeps the meaning it had before the field, and the version stays 0.

`firmware.elf` names a 32-bit little-endian Xtensa ELF executable. Parse loads each loadable segment with a nonzero memory size. A segment must lie inside one SRAM window, from `0x4037_0000` to `0x403e_0000` on the instruction bus or from `0x3fc8_8000` to `0x3fd0_0000` on the data bus, and no two segments may reach the same byte of SRAM through either window. The entry must lie in a segment on the instruction bus. A blob that is not such an executable is `netlist.firmware` rule `elf`, a segment or an entry outside those windows is rule `unmapped`, and two segments on one byte are rule `overlap`, each with `line` `null`. The function symbols and untyped labels of the symbol table name a PC in a diagnostic.

`firmware.flash` places each region's blob on the 16 MB flash at its `offset`, a byte count from the flash's start, and every byte no region places reads `0xFF`. A region past the end of the flash, or on a byte an earlier region placed, is `netlist.firmware` rule `region`. The partition table is at `0x8000`. The app is the factory app partition, else `ota_0`, as ESP-IDF's second-stage bootloader picks it while otadata is blank. The app partition must hold an ESP-IDF app image for the ESP32-S3, chip ID 9, whose checksum holds, and whose SHA-256 holds when its header says one follows. A flash with no partition table, no such app partition, a partition past the flash's end, otadata that is not blank, or an image that breaks one of those rules is rule `image`. The table's MD5 row is not checked. Each image segment must lie inside the instruction or data cache window, `0x4200_0000` to `0x4400_0000` or `0x3c00_0000` to `0x3e00_0000`, an SRAM window, or RTC fast or slow memory, and the entry must lie in a segment in the instruction-bus SRAM window, else rule `unmapped`. Each of these has `line` `null`. The image has no symbol table here, so a diagnostic's `symbol` is `null`.

`console` may be absent. Each event is bytes the host sends the console at `t_ps`. An event at t = 0, at or before the previous event, or with no bytes is `netlist.schedule` on field `console`, with rule `at_zero`, `not_increasing` or `same_state`. A byte above 255 is `netlist.syntax`. Each byte reaches the chip at the first tick at or after `t_ps`, as [esp32s3.md](esp32s3.md#what-it-models) says. `packages/engine/tests/circuits/esp32s3-screen` sends `g`, the byte 103, at 100 ms, the byte its app waits for before it draws:

```json
"console": [{ "t_ps": 100000000000, "bytes": [103] }]
```

`rom` names the mask ROM's ELF, which Espressif publishes for each chip revision. The model runs one ROM, revision 0's, so `rom` must have its SHA-256 and length, `c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd` and 949,552 bytes. Another value, `rom` beside `firmware.elf`, or no `rom` beside `firmware.flash` is rule `rom`. The catalog part `esp32s3-qfn56` commits that ELF as the default of its `rom` param. No engine example commits it. `packages/engine/scripts/blobs.sh` copies the committed file into a cache folder by pinned hash when the cache lacks a good copy, and fetches Espressif's release only when that file is absent or does not hash to the pinned value. It prints the folder for `--blobs`.

Each GPIO with a net drives high through `output.high_resistance` from `VCC`, drives low through `output.low_resistance` into `GND`, pulls up through `pull_up_resistance` to `VCC`, pulls down through `pull_down_resistance` to `GND`, pulls both ways through both, or is off, as the firmware sets it. In each of those states it also leaks through `leakage_resistance` to `VCC` and through another to `GND`. It reads high above `input_threshold.high` times `V(VCC) - V(GND)` over `GND`, and low at or below `input_threshold.low` times it.

A GPIO from `GPIO1` to `GPIO20` with a net is also an ADC input: channel n of ADC1 is `GPIO{n+1}` and channel n of ADC2 is `GPIO{n+11}`. A conversion reads `V(pin) - V(GND)` at the store that starts it and gives the code the chip's calibration maps back to that voltage, as [esp32s3.md](esp32s3.md#the-sar-adcs) says. The pad draws no current for it: it stamps only its drive, pulls and leakage. A conversion of a pad with no net ends the run with `chip.unsupported_setting`, `SENS` `pad_no_net`.

`efuse` names the eFuse set the chip's read registers hold, which sets its MAC, its revision and the ADC calibration IDF reads. `packages/socs/espressif/src/s3/efuse.txt` holds the sets, one section each. `synthetic` is the default. `format-test` is not a board: it is a made-up summary excerpt the parser test reads. Another name is `netlist.syntax`, with a message that lists the sets. A section that does not parse, because it names a field that is not in the csv or a calculated name, is `netlist.syntax` too, and the message names the field. A netlist without the field keeps the meaning it had before it, and the version stays 0. [esp32s3.md](esp32s3.md#the-sar-adcs) says what the synthetic set's calibration reads. A board's set is a new section in `efuse.txt`. It needs no code. For each field, copy the raw unsigned value `espefuse summary` prints in parentheses, in decimal, `0x` hex or `0b` binary. Copy `MAC` as the address summary prints. Copy `PSRAM_CAP` and `PSRAM_CAP_3` as summary prints those two lines. Do not copy `PSRAM_CAPACITY` or `WAFER_VERSION_MINOR`: espefuse calculates them (`PSRAM_CAP_3 << 2 | PSRAM_CAP`, and `WAFER_VERSION_MINOR_HI << 3 | WAFER_VERSION_MINOR_LO`) and they are not fields IDF reads. An ADC field's meaning column is sign-and-magnitude, and `adc_info` decodes it that way; raw `0x81` is 129 to IDF and `-4` to `adc_info`, so copy `0x81`.

```json
"efuse": "synthetic"
```

The example's values are Table 5-4 of the ESP32-S3 Series Datasheet v2.2, at 3.3 V and 25 C.

| Field | Value | Reading |
| --- | --- | --- |
| `output.high_resistance` | 16.5 ohm | `IOH` 40 mA at `VOH` 2.64 V, 0.8 VDD, with drive strength 3, so 0.66 V over 40 mA. |
| `output.low_resistance` | 17.68 ohm | `IOL` 28 mA at `VOL` 0.495 V with drive strength 3, so 0.495 V over 28 mA. |
| `output.*_drop` | 0 V | Each stage is a resistance from its rail. |
| `pull_up_resistance` | 45 kohm | `RPU`, typical. |
| `pull_down_resistance` | 45 kohm | `RPD`, typical. The example leaves it out, and `packages/engine/tests/circuits/esp32s3-pulls` names it. |
| `leakage_resistance` | 66 Mohm | `IIH` and `IIL` at most 50 nA, so 3.3 V over 50 nA. |
| `input_threshold` | 0.25 and 0.75 | `VIL` at most 0.25 VDD and `VIH` at least 0.75 VDD. |

Each GPIO with a net is a DC path to `VCC` and to `GND` in every drive state, through its leakage. An `esp32s3` has no `ratings` object and no `i`, `v`, or `p`. Its logic signals are its GPIOs with nets, each the level that pin reads, such as `#u1.logic.GPIO21`. Its console writes `traces/serial.csv` as usart `USB_SERIAL_JTAG`, and each UART byte as `UART0`, `UART1` or `UART2`. Its SPI2 sends each transfer as a `Spi` record with the nets of the GPIOs the matrix routed its CS, SCLK and data lines to, as [esp32s3.md](esp32s3.md#behind-the-seam) says, so an `st77916` on those nets takes it. `traces/link.csv` holds the records, and the file is written for every `esp32s3`, as the header alone when SPI2 sent none.

### Blobs

A blob is an input file the document names instead of holding, such as a firmware image. It is `{ "sha256", "bytes" }`. `sha256` is the file's SHA-256 as 64 lowercase hex digits, and `bytes` is its length, an unsigned integer. The document never holds a blob's contents.

A blob is a file named by its SHA-256. For each blob the netlist names, `orbe-run run` looks in the `blobs` folder beside the netlist file, then in each `--blobs <dir>` in the order given, and nowhere else. The first file under that name whose bytes hash to it is the blob, so a bad copy in one folder does not hide a good copy in another. `orbe-run run` reads no file whose name the netlist does not name. A file under a name the netlist names that cannot be read ends the run with `io_error` only when no folder has a good copy. A `--blobs` folder that cannot be listed ends it with `io_error` too.

A blob the document names that no folder has, a name whose every file hashes to another digest, and a blob whose length is not `bytes` are each `netlist.blob`, as [diagnostics.md](diagnostics.md#netlistblob) says. The run folder copies no blob. `manifest.json` lists each one, as [run-folder.md](run-folder.md#manifestjson) says.

## Ratings

A rating bounds the stress on a part. Every key of a kind's `ratings` object is required. Each value is a value object, or `null` for a part that is deliberately unrated. The `ratings` object is required on every kind but `atmega328p`, `voltage_source` and `capacitor`, where it is optional.

| Kind | Keys | Checked quantity |
| --- | --- | --- |
| `resistor` | `power` | `#<part>.p` |
| `diode`, `led`, `zener` | `current`, `power` | `#<part>.i`, `#<part>.p` |
| `linear_regulator` | `current`, `power` | `#<part>.iout`, `#<part>.p` |
| `npn`, `pnp` | `current`, `power` | `#<part>.ic`, `#<part>.p` |
| `nmos`, `pmos` | `current`, `power` | `#<part>.id`, `#<part>.p` |
| `voltage_source` | `current` | `#<part>.i` |
| `capacitor` | `reverse_voltage`, optional | pin `a` over pin `b` |
| `atmega328p` | `pin_current`, `supply_current`, `rail_margin`, `reset_voltage`, `supply_voltage` | each port pin's current, the current through `VCC` and `GND_8`, and each pin's voltage over `GND_8`, each named `#<part>.pin.<pin>` |
| `current_source`, `inductor`, `relay`, `switch`, `ne555`, `potentiometer`, `spi_replay`, `st77916`, `cst816`, `pulses_replay`, `ws2812`, `dht22` | none | |

`current`, `pin_current` and `supply_current` are in `A`. `power` is in `W`. `rail_margin`, `reset_voltage`, `supply_voltage` and `reverse_voltage` are in `V`. A rating must be greater than 0, except a capacitor's `reverse_voltage`, which may be 0. A reverse rating of 0 warns on any reverse voltage, and a pin exactly at pin `b` is inside. A missing key, an unknown key, and a missing `ratings` object where it is required are `netlist.syntax`. A wrong unit is `netlist.unit` with field `ratings.<key>`, such as `ratings.current`. A value of 0 or below is `netlist.value` with the same field, except a `reverse_voltage` of 0. A negative `reverse_voltage` is `netlist.value` with rule `non_negative`.

```json
"ratings": { "power": { "value": 0.25, "unit": "W" } }
```

The run compares the magnitude of each rated quantity with its rating, and each rated pin's voltage with its window. A DC rating, an `atmega328p`'s `pin_current` and `supply_current` and a `voltage_source`'s `current`, is compared through the quantity's RMS over the last 10.2 ms. A power rating, on every kind that has one, is compared through the average of its magnitude over that same window. Each window starts at 0 and fills from the first step after t = 0, and the operating point at t = 0 is no sample. A value and its rating are compared rounded to 3 significant digits. Going over is a warning, `rating.current`, `rating.power` or `rating.voltage`, in [diagnostics.md](diagnostics.md). A session also lists a windowed rating whose latest value is over it before the window crosses, as [over now](diagnostics.md#over-now).

## Sign rules

For every two-terminal part, pin 0 is the first pin listed above.

`i` is the current entering pin 0.

`v` is the voltage of pin 0 minus the voltage of pin 1.

`p` is `v * i`, the power the part absorbs. A source that delivers power has negative `i` and negative `p`.

For a diode or LED, `i` is the current through `Rs`, and `p` includes the power in `Rs`. The junction's internal node is not observed. For a zener, `i` is the current through `series_resistance`, and `p` includes both resistances. Its internal nodes are not observed. In breakdown `i` and `v` are negative.

For a capacitor, `i` is the solved current into pin `a`. For an inductor, `i` is the solved current into pin `a`, through its series resistance and its inductance, and `v` is across both. For a relay, `i` is the coil's current into `coil_a`, and `v` is from `coil_a` to `coil_b`. For a switch, `i` is `v` over the resistance of the contact on the interval that ends at that time. For a current source, `i` is minus its `current` on the interval that ends at that time, since that current leaves at `p`.

On the divider, `#r1.i` and `#r2.i` are positive, `#r1.p` and `#r2.p` are positive, and `#v1.i` and `#v1.p` are negative. On `packages/engine/tests/circuits/i-divider`, `#i1.i` is -0.001 and `#i1.p` is negative.

For a bipolar transistor, `ib` is the current entering the base pin and `ic` the current entering the collector pin. Each flows through its series resistance, so it includes the capacitors' current. `p` is `(V(collector) - V(emitter)) * ic + (V(base) - V(emitter)) * ib`, the power entering the part at its pins. A bipolar transistor has no `v`, and its internal nodes are not observed. A conducting PNP has negative `ib` and `ic`. On `packages/engine/tests/circuits/npn-led`, `#q1.ib` and `#q1.ic` are positive. On `packages/engine/tests/circuits/pnp-load` they are negative, and `#q1.p` is positive on both.

For an `nmos` or a `pmos`, `id` is the current entering the drain pin. It includes the drain-source capacitor and the gate-drain capacitor. `p` is `(V(drain) - V(source)) * id + (V(gate) - V(source)) * ig`, where `ig` is the current entering the gate pin. The gate current is not a separate ref. At DC the capacitors are open, so `ig` is 0 and `p` is `(V(drain) - V(source)) * id`. A MOSFET has no `v` and no internal node. On `packages/engine/tests/circuits/nmos-led`, `#m1.id` and `#m1.p` are positive. A conducting `pmos` has negative `id` when it sources current into a ground load. On `packages/engine/tests/circuits/pmos-high-side`, `#m1.id` is negative while the switch is on, and `#m1.p` is positive.

A linear regulator has no `i` and no `v`. `iin` is the current entering `in` and `iout` is the current entering `out`. While the part supplies a load, `iout` is negative. `p` is `(V(in) - V(gnd)) * iin + (V(out) - V(gnd)) * iout`. On `packages/engine/tests/circuits/regulator-3v3`, `#u1.iout` is negative and `#u1.p` is positive.

A potentiometer has no `i`, `v`, or `p`. Its pins and nets are observed as every part's are.

## Refs

A ref names one net, part, pin, quantity, or logic signal.

```text
ref      = "#" ( "net." name | "world." world | name [ "." ( quantity | "pin." name | "logic." name ) ] )
quantity = "ib" | "ic" | "id" | "iin" | "iout" | "i" | "v" | "p"
world    = "temperature" | "humidity" | "light"
```

`#<part>.usart.<name>`, such as `#u1.usart.USART0`, is reserved and is not a ref.

| Text | Meaning |
| --- | --- |
| `#net.vout` | Net `vout` |
| `#r1` | Part `r1` |
| `#r1.pin.a` | Pin `a` of `r1` |
| `#r1.pin.p` | Pin `p` of `r1`. Power of `r1` is `#r1.p`. |
| `#r1.i` | Current of `r1` |
| `#r1.v` | Voltage of `r1` |
| `#r1.p` | Power of `r1` |
| `#q1.ib` | Base current of transistor `q1` |
| `#q1.ic` | Collector current of transistor `q1` |
| `#m1.id` | Drain current of MOSFET `m1` |
| `#u1.iin` | Current entering `in` of regulator `u1` |
| `#u1.iout` | Current entering `out` of regulator `u1` |
| `#world.light` | The world's `light` |
| `#u1.logic.out` | Logic level of signal `out` of chip `u1` |
| `#k1.logic.contact` | Logic level of the contact of relay `k1` |

These texts parse and format back to the same text. The following texts are rejected.

- `r1`, because the `#` is missing
- `#`, `#net.`, `#r1.pin`, `#r1.pin.`, `#r1.`
- `#net.1vout`, `#1r`, because the name is illegal
- `#r1.x`, `#r1.i.v`

`#net.vout` is a net. `#net` is a part named `net`. `#world.light` is the world's light, and `#world` and `#world.i` are a part named `world`, since no world quantity shares a part quantity's name. `#u1.logic.out` is the level the chip drives on OUT, and `#u1.pin.out` is the pin itself. An NE555's signals are `out`, `reset`, `thres`, and `trig`. A relay's one signal is `contact`, high while the contact is closed. An ATmega328P's are named for its package pins: `PB0` to `PB5`, `PC0` to `PC6`, and `PD0` to `PD7`, each the level that pin reads, and `VCC` while `BODLEVEL` enables brown-out, the level its brown-out detector reads on the supply. `PB6` and `PB7` hold the crystal and have none. `#u1.logic.PB5` is the level PB5 reads, and `#u1.pin.PB5` is the pin itself. An ESP32-S3's are its GPIOs with nets, such as `GPIO21`.

Map keys and diagnostic order use this order: nets, then parts, then pins, then quantities, then world quantities in the table's order, then logic signals. Quantities sort `ib`, `ic`, `id`, `iin`, `iout`, `i`, `v`, then `p`. Names sort by byte order inside one group.

## Version

A version names a set of valid documents and what each one means. A change that keeps every valid document valid, with the same meaning, keeps the version. These keep it:

- a new part kind
- a new waveform kind
- a new analysis
- a new value of a string enum, such as a `frame`
- a rule that accepts more documents than before
- an optional field whose absence keeps an older document's meaning, such as `world`

Every other change bumps it. A new required field on an existing object bumps it, because an unknown field is an error and an older document lacks it.

The engine reads the versions in a list. This engine reads `[1]`. `netlist.version`'s `data.supported` is that list. A document whose `version` is not in the list yields only `netlist.version`. The engine reads an older listed version with that version's own rules, and gives it its old meaning.

Version 0 also gave every value a `source`, where the value came from. The solve never read it, so version 1 drops it. A part file still gives one, as the scene reader checks.
