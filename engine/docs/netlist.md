# Netlist v0

A netlist is fully resolved JSON. `version` is 0. [versions.md](versions.md) says which changes keep it. Every field is required, and an unknown field is an error. Nothing is filled in by default.

The netlist is the engine's only input. Data a part needs, such as firmware, lives inside the document.

Values are SI numbers. `unit` must already be the field's unit. Integer values are the exception. A time in `ps` sits on the engine's picosecond timeline, and a count is in `1`.

## Document

`examples/divider/netlist.json` is a 5 V source across `vin` and `gnd`, 1000 ohm from `vin` to `vout`, and 2000 ohm from `vout` to `gnd`.

```json
{
  "version": 0,
  "seed": 0,
  "analysis": { "op": {} },
  "ground": "gnd",
  "nets": ["gnd", "vin", "vout"],
  "parts": [
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "p": "vin", "n": "gnd" },
      "voltage": { "value": 5.0, "unit": "V", "source": "user" }
    },
    {
      "name": "r1",
      "kind": "resistor",
      "pins": { "a": "vin", "b": "vout" },
      "resistance": {
        "value": 1000.0,
        "unit": "ohm",
        "source": {
          "datasheet": {
            "doc": "resistor-nominal",
            "at": "Table 1, nominal",
            "basis": "nominal"
          }
        }
      },
      "ratings": { "power": null }
    },
    {
      "name": "r2",
      "kind": "resistor",
      "pins": { "a": "vout", "b": "gnd" },
      "resistance": { "value": 2000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    }
  ]
}
```

## Fields

`version` is `0`. Any number this engine does not read yields only `netlist.version`, even when the rest of the document is valid.

`seed` is an unsigned integer. The manifest records it. It moves switch bounce edges and nothing else. An operating point, and a run whose every `bounce` is `null`, do not depend on it.

`analysis` is `{"op": {}}` or `{"transient": {...}}`, described in [Transient analysis](#transient-analysis). The `op` object must be empty. A string such as `"op"` is `netlist.syntax`.

`ground` names one entry of `nets`. That net is held at 0 V. A name absent from `nets` is `netlist.ground`.

`nets` lists every net name. `parts` lists every part.

The engine stores ground first and sorts the other nets by name. It sorts parts by name. Reordering `nets` or `parts` in the JSON does not change observations or diagnostics.

## Transient analysis

`examples/rc-switch/netlist.json` charges a 1 uF capacitor through a switch and 10 kohm, with a 10 kohm bleeder, then lets it discharge.

```json
"analysis": { "transient": { "stop_ps": 81000000000, "max_step_ps": 20000000, "grid_ps": 20000000 } }
```

| Field | Meaning |
| --- | --- |
| `stop_ps` | The run ends at this time, in picoseconds. |
| `max_step_ps` | No time step is longer, in picoseconds. |
| `grid_ps` | The trace has a row at every multiple of it, in picoseconds. |

All three are JSON integers, like `seed`. They are the scenario's choices, so they carry no unit and no source. A float, a negative number, or a missing field is `netlist.syntax`. `stop_ps` must be greater than 0. `max_step_ps` must be at least 1000 and at most `stop_ps`. `grid_ps` must be at least 1000 and at most `stop_ps`. A break is `netlist.analysis` with the field and the rule.

The rows are t = 0, every multiple of `grid_ps` before `stop_ps`, and `stop_ps`. That is `ceil(stop_ps / grid_ps) + 1` rows. `stop_ps` need not be a multiple of `grid_ps`. The engine lands a step on every row time, so the longest step is at most the smaller of `max_step_ps` and `grid_ps`. A row is a point sample of the solution at its time. Between rows the trace says nothing. [run-folder.md](run-folder.md#traces) says how to pick `grid_ps`. A dense switch or source schedule costs steps, not rows, and can hit the run's step limit.

The run solves the operating point at t = 0 first, with every capacitor open or held at its `initial` voltage, every switch at its `initial` contact, and every source at its value at t = 0, which is a waveform's `initial`. That is the initial condition. Then it steps to `stop_ps`. Every switch edge and every source jump before `stop_ps` takes effect at its picosecond. [engine.md](engine.md) describes how the engine steps there, how it finds chip events, and how accurate it is.

## Names

A name is an ASCII letter, then ASCII letters, digits, or `_`, from 1 to 64 bytes.

A part named `net` is `netlist.name` with problem `reserved`. Pin, unit, and value checks on that part are still reported. A name that fails the grammar is `netlist.name` and those checks are not run, because the part has no ref. A net may be named `net`.

Two nets, or two parts, with the same name are `netlist.duplicate`. The ref is `#net.<name>` or `#<part>`. Pin, unit, and value checks on a duplicate part are still reported.

## Parts

`kind` selects the part. Pin keys must be exactly that kind's pin names. A missing or extra key is `netlist.pins`. An extra key that is a legal name and names a net absent from `nets` is also `netlist.unknown_net`.

### Resistor

`kind` is `resistor`. Pins are `a` and `b`, in that order for the sign rule.

`resistance.value` is in `ohm`, greater than 0, and its reciprocal must be finite. Zero or a negative value is `netlist.value` with rule `positive`. A positive value whose reciprocal overflows is `netlist.value` with rule `finite_reciprocal`. A different unit is `netlist.unit`. A number outside the f64 range is `netlist.syntax`.

`ratings` holds `power`. See [Ratings](#ratings).

### Voltage source

`kind` is `voltage_source`. Pins are `p` and `n`. The source sets `V(p) - V(n)` equal to `voltage`.

`voltage` is a value in `V`, which may be any finite number, including 0 and negative values. Or it is a waveform in `V`, described in [Source waveforms](#source-waveforms). A different unit is `netlist.unit`.

A voltage source has no `ratings` field.

### Current source

`kind` is `current_source`. Pins are `p` and `n`. The source's `current` leaves it at `p`, flows through the circuit, and returns at `n`. `examples/i-divider` drives 1 mA into 1 kohm parallel to 2 kohm.

```json
{
  "name": "i1",
  "kind": "current_source",
  "pins": { "p": "vt", "n": "gnd" },
  "current": { "value": 0.001, "unit": "A", "source": "user" }
}
```

`current` is a value in `A`, any finite number, or a waveform in `A`. A different unit is `netlist.unit`. Source stepping scales it, as it scales a voltage source.

A current source has no `ratings` field. It joins no DC path. A net reached only through current sources and capacitors without an `initial` voltage is `net.floating`, and only gmin holds it: 1 mA into such a net reads 1e9 V. A resistor to ground closes the loop.

### Source waveforms

A source's `voltage` or `current` is a value object, which holds at every time, or an object with `kind`, which changes. Its levels are values in the source's unit, any finite number. Its times are JSON integers in picoseconds, like `stop_ps`. They are the scenario's choices, so they carry no unit and no source.

A step holds `initial` until `at_ps`, then `final`.

```json
"voltage": {
  "kind": "step",
  "initial": { "value": 0.0, "unit": "V", "source": "user" },
  "final": { "value": 5.0, "unit": "V", "source": "user" },
  "at_ps": 1000000000
}
```

A pulse holds `initial` until `delay_ps`, then `pulsed` for `width_ps`, then `initial` until the next period, every `period_ps`. `examples/current-pulse` uses this one. A single pulse is one whose next rise is past `stop_ps`.

```json
"current": {
  "kind": "pulse",
  "initial": { "value": 0.0002, "unit": "A", "source": "user" },
  "pulsed": { "value": 0.001, "unit": "A", "source": "user" },
  "delay_ps": 1000000000,
  "width_ps": 2000000000,
  "period_ps": 5000000000
}
```

Every change is an ideal jump. The level on the interval that ends at a jump is the old one, so a trace row at the time of a jump holds it. A jump at `stop_ps` leaves the last row on the old level. The operating point holds `initial`. An operating point run reads only that.

A time that breaks a rule is `netlist.waveform`, with its dotted field, such as `current.width_ps`, and the rule. Every broken rule is reported.

- `at_zero`: `at_ps` or `delay_ps` is 0. The operating point already holds `initial` there.
- `too_close`: `width_ps` is under 2000, or `period_ps` is under `width_ps` plus 2000. Two jumps of one source are at least 2 ns apart, as two edges of one switch are.
- `overflow`: `delay_ps` plus `width_ps` does not fit in an unsigned 64-bit integer.

A jump between equal levels is still a jump. A missing or unknown key, a float or negative time, a `kind` other than `step` or `pulse`, and an object with both `value` and `kind` are `netlist.syntax`. A level in the wrong unit is `netlist.unit` with its dotted field, such as `current.pulsed`.

### Capacitor

`kind` is `capacitor`. Pins are `a` and `b`.

`capacitance.value` is in `F`. It follows the resistance rules. It must be greater than 0, and its reciprocal must be finite.

`initial` is required. It is a value in `V`, any finite number, or `null`.

```json
"initial": { "value": 0.0, "unit": "V", "source": "user" }
```

A capacitor with `"initial": null` is open at the operating point, so `#c1.i` is 0 there. A capacitor with a value is held there, behind a series resistance of 1 fs divided by its capacitance. That resistance is the backward Euler companion of a 1 fs step. `V(a) - V(b)` is the value plus `#c1.i` times the resistance, and `#c1.i` is the current the hold takes. For 1 uF and 5 mA the voltage moves by 5 pV. A held capacitor across a voltage source, held capacitors in parallel, and held capacitors in series across a source then solve instead of making the matrix singular. Capacitors in parallel share their current by capacitance. Two in parallel at different values meet at their capacitance-weighted mean, as their charge would. A held value that disagrees with a voltage source across it draws the difference over the resistance, 1e9 A per volt for 1 uF. That current also enters the error estimate of the first steps, so they fall to the 1 ps floor and grow back. A missing key is `netlist.syntax`. A wrong unit is `netlist.unit` with field `initial`. In a step the current follows from the voltage across it and the last accepted point.

A capacitor has no `ratings` field.

### Switch

`kind` is `switch`. Pins are `a` and `b`. It is a resistor whose value follows a contact over time.

```json
{
  "name": "sw1",
  "kind": "switch",
  "pins": { "a": "vbtn", "b": "gnd" },
  "on_resistance": { "value": 0.1, "unit": "ohm", "source": "user" },
  "off_resistance": { "value": 1000000000.0, "unit": "ohm", "source": "user" },
  "initial": "open",
  "events": [
    { "t_ps": 2000000000, "state": "closed" },
    { "t_ps": 12000000000, "state": "open" }
  ],
  "bounce": {
    "window": { "value": 1000000000, "unit": "ps", "source": "user" },
    "max_bounces": { "value": 4, "unit": "1", "source": "user" }
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

Bounce is a fixed, integer-only expansion. Event `k` of switch `name` draws from SplitMix64 seeded with `mix(mix(seed ^ fnv1a64(name)) ^ k)`, where `mix` is SplitMix64's output function and `fnv1a64` is 64-bit FNV-1a over the name's bytes. It draws a count `n` from 1 to `max_bounces`, then `2n` values from 0 to `window - 2n * 2000`, sorted. Draw `j`, counted from 1, plus `j * 2000` is the offset of the `j`-th flip after the event. So the contact flips `2n` times within `window` and ends at the event's state. A draw below `m` is the high 64 bits of a 64-bit output times `m`. `scripts/ngspice_refs.py` implements the same steps. The offsets are uniform, which is enough to test exact breakpoints and determinism. They are not a contact physics model.

The contact on the interval that ends at time `t` is the state after every edge strictly before `t`. A trace row at the time of an edge therefore holds the old contact. An edge at `stop_ps` leaves the last row on the old contact. `observations.json` lists every edge at its picosecond.

A switch has no `ratings` field. A switch is a DC path in either state, because its off resistance is finite.

### Diode and LED

`kind` is `diode` or `led`. Both take the same fields and solve the same equation. Pins are `anode` and `cathode`.

The part is a junction in series with a resistance. The junction current is `Is * (exp(vj / (N * kT/q)) - 1)`. `V(anode) - V(cathode)` is `vj` plus the current times `Rs`.

| Field | Symbol | Unit |
| --- | --- | --- |
| `saturation_current` | `Is` | `A` |
| `emission_coefficient` | `N` | `1` |
| `series_resistance` | `Rs` | `ohm` |

Each value must be greater than 0, and its reciprocal must be finite. Those are the resistor's rules. `n*VT` must also be a normal number, and `vcrit` must be finite and positive. `VT` is `k*T/q` at 298.15 K with the exact SI values of `k` and `q`. `vcrit` is `n*VT*ln(n*VT/(sqrt(2)*Is))`, which is positive when `n*VT > sqrt(2)*Is`. A junction that breaks either condition is `netlist.value` with rule `finite_junction`. The field is `emission_coefficient` when `n*VT` is not normal. It is also `emission_coefficient` when `vcrit` is not finite and positive and `Is` is a normal number. The field is `saturation_current` when `vcrit` is not finite and positive and `Is` is not a normal number.

The device is at 25 C, 298.15 K. `k` and `q` are the exact SI values. There is no temperature field and no temperature scaling. A value fitted at another temperature does not describe the part here.

`ratings` holds `current` and `power`.

`examples/led-5v` and `examples/led-9v` use a Kingbright WP7113ID, a red 5 mm LED. Its `Is`, `N`, and `Rs` solve the equation exactly through three points read off the datasheet's forward current figure at 25 C: (1.70 V, 2 mA), (1.90 V, 10 mA), and (2.00 V, 20 mA). The fit gives `Is` = 6.711e-10 A, `N` = 4.427, and `Rs` = 2.115 ohm. Each value's `at` names the figure, the points, and the temperature, so the fit can be redone from the citation.

### NE555

`kind` is `ne555`. Pins are `gnd`, `trig`, `out`, `reset`, `cont`, `thres`, `disch` and `vcc`, in package order. `examples/ne555-led` is the astable of TI SLFS022K Figure 6-5, 10 kohm and 47 kohm with 10 uF, driving an LED through 220 ohm. Its values cite SLFS022K, and its ladder cites the schematic of the pin-compatible LM555, TI SNAS548D.

```json
{
  "name": "u1",
  "kind": "ne555",
  "pins": {
    "gnd": "gnd", "trig": "thr", "out": "out", "reset": "vcc",
    "cont": "cont", "thres": "thr", "disch": "dis", "vcc": "vcc"
  },
  "ladder_resistance": { "value": 5000.0, "unit": "ohm", "source": "user" },
  "reset_threshold": { "value": 0.7, "unit": "V", "source": "user" },
  "discharge": {
    "on_resistance": { "value": 18.75, "unit": "ohm", "source": "user" },
    "off_resistance": { "value": 250000000.0, "unit": "ohm", "source": "user" }
  },
  "supply_resistance": { "value": 1875.0, "unit": "ohm", "source": "user" },
  "output": {
    "high_drop": { "value": 0.9, "unit": "V", "source": "user" },
    "high_resistance": { "value": 8.0, "unit": "ohm", "source": "user" },
    "low_drop": { "value": 0.016666666666666666, "unit": "V", "source": "user" },
    "low_resistance": { "value": 16.666666666666668, "unit": "ohm", "source": "user" }
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

## Ratings

A rating bounds the stress on a part. Every key of a kind's `ratings` object is required. Each value is a value object, or `null` for a part that is deliberately unrated.

| Kind | Keys | Checked quantity |
| --- | --- | --- |
| `resistor` | `power` | `#<part>.p` |
| `diode`, `led` | `current`, `power` | `#<part>.i`, `#<part>.p` |
| `voltage_source`, `current_source`, `capacitor`, `switch`, `ne555` | none | |

`current` is in `A`. `power` is in `W`. A rating must be greater than 0. A missing key, an unknown key, and a missing `ratings` object are `netlist.syntax`. A wrong unit is `netlist.unit` with field `ratings.current` or `ratings.power`. A value of 0 or below is `netlist.value` with the same field.

```json
"ratings": { "power": { "value": 0.25, "unit": "W", "source": "user" } }
```

The run compares the magnitude of each rated quantity with its rating. Going over is a warning, `rating.current` or `rating.power`, in [diagnostics.md](diagnostics.md).

## Sources

`source` is one of three forms. An integer value such as `bounce.window` takes the same forms.

- `"user"`
- `{"closed_form": {"note": "<text>"}}`
- `{"datasheet": {"doc": "<id>", "at": "<place>", "basis": "<basis>"}}`

`note`, `doc`, and `at` are strings. `basis` says which figure of the datasheet the value is. Any other string is `netlist.syntax`.

| `basis` | The value is |
| --- | --- |
| `typical` | a typical column, or read off a typical-only curve |
| `min` | a minimum column |
| `max` | a maximum column, recommended operating conditions included |
| `nominal` | the part's stated value, such as a schematic value or a marked resistance |
| `absolute_max` | an absolute maximum rating |

The solve uses `value` alone.

## Sign rules

For every two-terminal part, pin 0 is the first pin listed above.

`i` is the current entering pin 0.

`v` is the voltage of pin 0 minus the voltage of pin 1.

`p` is `v * i`, the power the part absorbs. A source that delivers power has negative `i` and negative `p`.

For a diode or LED, `i` is the current through `Rs`, and `p` includes the power in `Rs`. The junction's internal node is not observed.

For a capacitor, `i` is the solved current into pin `a`. For a switch, `i` is `v` over the resistance of the contact on the interval that ends at that time. For a current source, `i` is minus its `current` on the interval that ends at that time, since that current leaves at `p`.

On the divider, `#r1.i` and `#r2.i` are positive, `#r1.p` and `#r2.p` are positive, and `#v1.i` and `#v1.p` are negative. On `examples/i-divider`, `#i1.i` is -0.001 and `#i1.p` is negative.

## Refs

A ref names one net, part, pin, quantity, or logic signal.

```text
ref      = "#" ( "net." name | name [ "." ( quantity | "pin." name | "logic." name ) ] )
quantity = "i" | "v" | "p"
```

| Text | Meaning |
| --- | --- |
| `#net.vout` | Net `vout` |
| `#r1` | Part `r1` |
| `#r1.pin.a` | Pin `a` of `r1` |
| `#r1.pin.p` | Pin `p` of `r1`. Power of `r1` is `#r1.p`. |
| `#r1.i` | Current of `r1` |
| `#r1.v` | Voltage of `r1` |
| `#r1.p` | Power of `r1` |
| `#u1.logic.out` | Logic level of signal `out` of chip `u1` |

These texts parse and format back to the same text. The following texts are rejected.

- `r1`, because the `#` is missing
- `#`, `#net.`, `#r1.pin`, `#r1.pin.`, `#r1.`
- `#net.1vout`, `#1r`, because the name is illegal
- `#r1.x`, `#r1.i.v`

`#net.vout` is a net. `#net` is a part named `net`. `#u1.logic.out` is the level the chip drives on OUT, and `#u1.pin.out` is the pin itself. An NE555's signals are `out`, `reset`, `thres`, and `trig`.

Map keys and diagnostic order use this order: nets, then parts, then pins, then quantities, then logic signals. Quantities sort `i`, then `v`, then `p`. Names sort by byte order inside one group.
