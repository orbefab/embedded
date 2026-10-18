# Netlist v0

A netlist is fully resolved JSON. `version` is 0. Every field is required, and an unknown field is an error. Nothing is filled in by default.

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
            "typical": false
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

`version` is `0`. Any other number yields only `netlist.version`, even when the rest of the document is valid.

`seed` is an unsigned integer. The manifest records it. It moves switch bounce edges and nothing else. An operating point, and a run whose every `bounce` is `null`, do not depend on it.

`analysis` is `{"op": {}}` or `{"transient": {...}}`, described in [Transient analysis](#transient-analysis). The `op` object must be empty. A string such as `"op"` is `netlist.syntax`.

`ground` names one entry of `nets`. That net is held at 0 V. A name absent from `nets` is `netlist.ground`.

`nets` lists every net name. `parts` lists every part.

The engine stores ground first and sorts the other nets by name. It sorts parts by name. Reordering `nets` or `parts` in the JSON does not change observations or diagnostics.

## Transient analysis

`examples/rc-switch/netlist.json` charges a 1 uF capacitor through a switch and 10 kohm, with a 10 kohm bleeder, then lets it discharge.

```json
"analysis": { "transient": { "stop_ps": 81000000000, "max_step_ps": 20000000 } }
```

| Field | Meaning |
| --- | --- |
| `stop_ps` | The run ends at this time, in picoseconds. |
| `max_step_ps` | No time step is longer, in picoseconds. |

Both are JSON integers, like `seed`. They are the scenario's choices, so they carry no unit and no source. A float, a negative number, or a missing field is `netlist.syntax`. `stop_ps` must be greater than 0. `max_step_ps` must be at least 1000 and at most `stop_ps`. A break is `netlist.analysis` with the field and the rule.

The run solves the operating point at t = 0 first, with every capacitor open or held at its `initial` voltage, and every switch at its `initial` contact. That is the initial condition. Then it steps to `stop_ps`. Every switch edge before `stop_ps` is a breakpoint. So is 1 ns after t = 0, and 1 ns after each edge, when that time is before `stop_ps`. A step lands on each breakpoint and on `stop_ps`.

At each switch edge the row holds the old contact. Then the run solves once more at the edge's time, with the new contacts in force and every capacitor held at its voltage, the way an `initial` voltage is held at t = 0. This restart solve gives the currents just after the edge. It is not a row, and the next step starts from it. Each held voltage moves by the capacitor's current times 1 fs over its capacitance.

The step policy is fixed. A segment starts at t = 0 and at every restart. Its first step wants 1 ns and is backward Euler. Every later step in it is trapezoidal, except on a stiff capacitor. Each step's local truncation error comes from the capacitor currents the solve already has. Backward Euler's is `h/2 * |i1 - i0| / C`. The trapezoidal rule's is `h^3 * |DD2(i)| / (6C)`, the second divided difference over the segment's last three points. The tolerance is `1e-6 * max(|v0|, |v1|) + 1e-9` V, Newton's own, on the voltage across each capacitor.

A step over its tolerance on any capacitor is rejected and retried from the same accepted point. The next try is `0.9 * ratio^(-1/(p+1))` of it, between 0.25 and 0.9, where `ratio` is the worst error over its tolerance and `p` is 1 if that capacitor took backward Euler and 2 if it took the trapezoidal rule. An accepted step grows by the same factor, at most 2, up to `max_step_ps`. When the factor is over 1 it grows by at least 1 ps. A backward Euler step of 1 ps is accepted whatever its error. A capacitor whose trapezoidal step of 1 ps is over its tolerance holds a mode faster than any step, on which the trapezoidal rule rings. It turns stiff, and the step is retried. A stiff capacitor takes backward Euler, which damps that mode. The other capacitors keep the trapezoidal rule. Once a stiff capacitor's accepted step reaches 256 ps, the next step tries it on the trapezoidal rule. The try holds only if each tried capacitor's error is under 1e-3 of its tolerance, because what is left of the mode does not decay on the trapezoidal rule. Otherwise the step is retried with them on backward Euler, and the next try waits for a step twice as long. A restart clears every stiff capacitor. An accepted step cut short by a breakpoint or by `stop_ps` keeps the step it wanted. A rejected one does not, and its retry grows from the shorter step. With no capacitor there is no error, so the step doubles. A step whose Newton solve fails is halved and retried from the same accepted point, down to 1 ps. A failure at 1 ps ends the run with `solver.convergence`.

### Chip events

A chip reads its inputs through watches. A watch is a sum of node voltages times fixed coefficients, minus an offset, and it reads high only above 0. The NE555's are TRIG minus its lower ladder node, THRES minus CONT, and RESET minus GND minus `reset_threshold`.

After each accepted step the run reads every watch on the step's solution. If any reads differently than at the last accepted point, the run locates the first picosecond where one does. Each probe re-solves the same step, from the same point with the same method, shortened to the probe time. The probe follows regula falsi on the earliest watch to flip, rounded to a whole picosecond inside the bracket. It turns to bisection once one end has moved twice in a row, and it stops at a 1 ps bracket. The located point is the later end, the first solved picosecond where a watch reads flipped, and it becomes the row. A regula falsi probe whose solve stalls is replaced by the bracket's midpoint. A midpoint that stalls ends the run with `solver.convergence`, and a singular probe ends it with `solver.singular`.

A watch cuts the step; only a stamp change restarts. At the located point each chip reacts to its levels. If a chip's new state stamps differently, or a switch edge falls there, the run restarts at that point the way it does at a switch edge. The row holds the chips' old states, and the held solve after it is not a row. A flip that changes no drive still gets its edge at its picosecond, and the segment, its method and its error history continue across it. In `examples/ne555-led`, THRES drops back under CONT 1 ps after the latch resets, which is such a flip.

Every watch that flips at one picosecond, on every chip, reacts at once against the levels before it, chips in part order. The run then reads the levels on the solve with the new states and reacts again, up to 8 solves. Chips that still change after the 8th are `chip.unsettled`. The logic levels recorded at the point are the ones the chips reacted to. A watch that flipped on the step keeps its flipped level there, and every other watch takes its level on the solve with the new states. When that solve is back across a flipped watch's threshold, the return is recorded on the next row. A monostable with THRES on DISCH does this, since the discharge pulls THRES back down through the hold. Every located event is at a later whole picosecond, so events cannot pile up at one time. An event located at `stop_ps`, or in the step that ends there, is recorded, and its row is the last. No restart runs at `stop_ps`.

The operating point at t = 0 settles each chip the same way. It starts from the chip's power-on state, the NE555's latch reset, and reacts to the levels it reads. Only the solve that converges at the nominal gmin and source scale is read, never an intermediate rung of the ladder.

Watches are read at accepted steps. A signal that crosses and crosses back inside one step is missed. The error control keeps steps short where a signal curves, and `max_step_ps` caps them.

Every chip kind keeps four rules. A watch reads node voltages, never a branch current. A chip's state holds only what its stamp reads, so every change of state restarts. A chip with memory that leaves its outputs alone would need its own rule for when to restart. A chip's DC paths are the union over its states, so a path that some state opens, such as a high-impedance output, still counts for `net.floating`. Chips have no propagation delay, so two that flip each other at one time can have no states they agree on. Two NE555s, each OUT on the other's TRIG, reset together, then both falling OUTs set both latches while both THRES are still high. That is `chip.unsettled`.

### Accuracy

`max_step_ps` caps the step. It sets how dense the trace is, not how accurate. Each step above the 1 ps floor holds every capacitor's local truncation error under 1e-6 of its voltage plus 1 nV, from a consistent restart after every edge. Error accumulates over steps. The RC example stays under 1e-6 of span of its closed form, and a 10 ns discharge after an edge under 2e-5.

A step at the floor is accepted over its tolerance, so a time constant under about 43 ps is not held to it. A discharge after an edge measures this, as its worst error over its swing:

| Time constant | Worst error |
| --- | --- |
| 1 ps | 0.13 |
| 10 ps | 1.8e-2 |
| 30 ps | 6.0e-3 |
| 50 ps | 1.7e-4 |
| 100 ps | 3.9e-5 |
| 300 ps | 1.1e-5 |
| 1 ns and up | 1.3e-5 |

Under about 43 ps the capacitor turns stiff, and the error is about 0.18 times 1 ps over the time constant. Above it the capacitor stays trapezoidal, and the error comes from its first step. The restart's hold shifts the start of the discharge by 1 fs over the time constant, as a fraction of the swing. A stiff capacitor whose voltage keeps moving after its mode dies goes back to the trapezoidal rule. 10 pF dumped through 2 ohm into 1 nF, then both charged through 1 Mohm, stays under 2e-5 of its closed form. So does a 10 ns discharge beside a 30 ps mode switched in. A capacitor whose step never reaches 256 ps stays stiff to the next restart. It keeps backward Euler's error, which accumulates with one sign to about 2e-4 of its swing over a few of its time constants. It also takes more rows, since that error grows with the square of the step. A capacitor on a separate node keeps its accuracy beside it, and one that shares its slow mode shares its error. Two 10 pF coupled through 1 ohm, with a common mode of 18 ns, take 55 ps steps and are 2e-4 of their swing off once the mode has died. Halving `max_step_ps` and comparing the traces is still a check for a circuit with no closed form.

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

`kind` is `voltage_source`. Pins are `p` and `n`. The source sets `V(p) - V(n)` equal to `voltage.value`.

`voltage.value` is in `V` and may be any finite number, including 0 and negative values. A different unit is `netlist.unit`.

A voltage source has no `ratings` field.

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

The contact on the interval that ends at time `t` is the state after every edge strictly before `t`. A run's trace row at an edge therefore holds the old contact, and the row at the earlier of 1 ns later and `stop_ps` holds the new one.

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
- TRIG reads high above the lower node, THRES above CONT, and RESET above GND by more than `reset_threshold`. Each is a watch, described in [Chip events](#chip-events). No comparator has an offset, hysteresis, or input current.
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

The latch's state is the chip's only state. It is not a netlist field. The operating point settles it, as described in [Chip events](#chip-events).

A chip has no `ratings` field and no `i`, `v`, or `p`. Its logic levels are refs of their own, described in [Refs](#refs). It is a DC path from VCC to CONT, CONT to GND, VCC to GND, OUT to VCC, OUT to GND, and DISCH to GND, in either latch state. A net reached only through TRIG, THRES, or RESET floats.

## Ratings

A rating bounds the stress on a part. Every key of a kind's `ratings` object is required. Each value is a value object, or `null` for a part that is deliberately unrated.

| Kind | Keys | Checked quantity |
| --- | --- | --- |
| `resistor` | `power` | `#<part>.p` |
| `diode`, `led` | `current`, `power` | `#<part>.i`, `#<part>.p` |
| `voltage_source`, `capacitor`, `switch`, `ne555` | none | |

`current` is in `A`. `power` is in `W`. A rating must be greater than 0. A missing key, an unknown key, and a missing `ratings` object are `netlist.syntax`. A wrong unit is `netlist.unit` with field `ratings.current` or `ratings.power`. A value of 0 or below is `netlist.value` with the same field.

```json
"ratings": { "power": { "value": 0.25, "unit": "W", "source": "user" } }
```

The run compares the magnitude of each rated quantity with its rating. Going over is a warning, `rating.current` or `rating.power`, in [diagnostics.md](diagnostics.md).

## Sources

`source` is one of three forms. An integer value such as `bounce.window` takes the same forms.

- `"user"`
- `{"closed_form": {"note": "<text>"}}`
- `{"datasheet": {"doc": "<id>", "at": "<place>", "typical": false}}`

`note`, `doc`, and `at` are strings. `typical` is a boolean. The solve uses `value` alone.

## Sign rules

For every two-terminal part, pin 0 is the first pin listed above.

`i` is the current entering pin 0.

`v` is the voltage of pin 0 minus the voltage of pin 1.

`p` is `v * i`, the power the part absorbs. A source that delivers power has negative `i` and negative `p`.

For a diode or LED, `i` is the current through `Rs`, and `p` includes the power in `Rs`. The junction's internal node is not observed.

For a capacitor, `i` is the solved current into pin `a`. For a switch, `i` is `v` over the resistance of the contact on the interval that ends at that time.

On the divider, `#r1.i` and `#r2.i` are positive, `#r1.p` and `#r2.p` are positive, and `#v1.i` and `#v1.p` are negative.

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
