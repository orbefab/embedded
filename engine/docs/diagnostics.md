# Diagnostics

A diagnostic is one JSON object. The fields are written in this order.

| Field | Meaning |
| --- | --- |
| `code` | Stable id, `word.word` with lowercase letters and `_` in the tail. |
| `severity` | `error` or `warning`. A reader accepts others. |
| `refs` | Refs into the netlist. Empty when the code names no net, part, or pin. |
| `t_ps` | Picoseconds from t = 0, or `null` when the finding is about the input. |
| `message` | One sentence for a person. |
| `data` | Extra fields. Keys are alphabetical. |

`code`, the condition it names, the shape of `refs`, the meaning of `t_ps`, and the keys and types of `data` are stable. A condition never moves to another code. `message` is for people and its text can change, so match on `code` and `data`. New codes, severities and `data` keys can appear, and a reader accepts them. [versions.md](versions.md#diagnostics) has the whole policy.

The run sorts diagnostics by `t_ps`, then `code`, then `refs`, then the JSON bytes of the whole object. A `null` time sorts before a number, so every input finding precedes a solver finding.

A session's `diagnostics()` returns the warnings that list would hold if the session ended now. The folder's warnings are `net.floating`, `trace.row_interpolated`, `rating.current` and `rating.power`. `diagnostics()` returns `net.floating`, `rating.current` and `rating.power`. It leaves out `trace.row_interpolated`, because that warning names a grid row whose probe failed twice, and only a recorded trace defines those rows. An open session has no grid. The accepted steps do not depend on the grid, and neither do the warnings `diagnostics()` returns. Every other code on this page is an error. The error that ends a run is `fault` on the session, and `diagnostics()` does not return it. [engine.md](engine.md#sessions) says when the samples are taken.

A run whose diagnostics are all warnings still writes the operating point. A run with an error writes the observations it reached, as [run-folder.md](run-folder.md#observationsjson) says. That is `{}` when nothing solved, as at t = 0. A transient keeps its grid rows before the error, and its last accepted step is the last row. Its observations end there. A warning found before that error is kept, including a rating sampled on any accepted step before it.

Stdout of `sim run` lists the same objects in the same order.

## netlist.syntax

Severity `error`. No refs. `t_ps` is `null`.

The message is:

```text
schema mismatch at `{path}` ({detail})
```

`data` holds `column`, `detail`, `line`, and `path`. `detail` is the parser text. A typo such as `resistence` lands here. A repeated pin key lands here, with that key in `detail`. Bytes after the JSON value land here. Trailing whitespace does not. Any other `version` is reported only as `netlist.version`, including when the rest of the document is broken.

## netlist.version

Severity `error`. No refs. `t_ps` is `null`.

The message is:

```text
netlist version {found} is not supported; this engine reads {supported}
```

`data` holds `found` and `supported`. `supported` lists every version this engine reads, `[0]`. The message joins that list with `, `.

## netlist.name

Severity `error`. No refs. `t_ps` is `null`.

The message is:

```text
name `{name}` at `{path}` is not valid ({reason})
```

`data` holds `name`, `path`, and `problem`. `problem` is `empty`, `too_long`, `bad_char`, or `reserved`.

`reserved` is a part named `net`. The name grammar is in [netlist.md](netlist.md).

## netlist.duplicate

Severity `error`. The `refs` array holds one ref, `#<part>` or `#net.<name>`. `t_ps` is `null`.

The message is `{ref} is declared more than once`.

`data` is `{}`.

## netlist.unknown_net

Severity `error`. Refs is the pin, for example `#r2.pin.b`. `t_ps` is `null`.

The message is:

```text
pin #r2.pin.b names net `gnd2`, which is not in `nets`
```

`data` holds `net`, the missing name.

## netlist.pins

Severity `error`. Refs is the part. `t_ps` is `null`.

The message is `{ref} pins do not match its kind`.

`data` holds `missing` and `unexpected`. `missing` follows the kind's pin order. `unexpected` is sorted by name.

## netlist.unit

Severity `error`. Refs is the part. `t_ps` is `null`.

The message is:

```text
{ref} {field} is in `{found}`; it must be in `{expected}` (SI base units)
```

`data` holds `expected`, `field`, and `found`. Resistance must be `ohm`. Voltage must be `V`. Current must be `A`. A source waveform's levels take the source's unit, with dotted fields such as `voltage.final` and `current.pulsed`. Capacitance must be `F`. A switch's `on_resistance` and `off_resistance` must be `ohm`, `bounce.window` must be `ps`, and `bounce.max_bounces` must be `1`. The diode and LED fields, and the `ratings.current` and `ratings.power` fields, take the units in [netlist.md](netlist.md).

## netlist.value

Severity `error`. Refs is the part. `t_ps` is `null`.

The message for `positive` is `{ref} {field} is {value}; it must be positive`.

The message for `finite_reciprocal` is `{ref} {field} is {value}; its reciprocal must be finite`.

The message for `finite_junction` is `{ref} {field} is {value}; n*VT must be normal, and vcrit must be finite and positive`.

The message for `bounce_room` is `{ref} {field} is {value}; it must fit 2 * max_bounces edges 2000 ps apart`.

The message for `non_negative` is `{ref} {field} is {value}; it must not be negative`.

The message for `fraction` is `{ref} {field} is {value}; it must be between 0 and 1`.

The message for `above_high` is `{ref} {field} is {value}; it must not be above high`.

The message for `hysteresis` is `{ref} {field} is {value}; it must be below pull_in_current`.

The message for `unsupported` is `{ref} {field} is {byte}; the model does not run that mode`, with the byte in hex, such as `0x62`.

`data` holds `field`, `rule`, and `value`. `rule` is `positive`, `finite_reciprocal`, `finite_junction`, `bounce_room`, `non_negative`, `fraction`, `above_high`, `hysteresis`, or `unsupported`. The message prints `value` in the same form as `data.value`, but for `unsupported`. Resistance must be positive, and its reciprocal must be finite. A value that breaks both reports `positive`. A potentiometer's `resistance` follows those rules. Its `position` must be inside (0, 1), which breaks `fraction`. A leg, `position * resistance` or `(1 - position) * resistance`, that breaks the resistance rules is `netlist.value` on field `resistance`, and `value` is that leg. A source's voltage or current, and each level of its waveform, only has to be finite and in its unit. The diode and LED fields follow the resistance rules. `saturation_current` and `emission_coefficient` also follow `finite_junction`, as [netlist.md](netlist.md) defines it. Every `npn` and `pnp` field follows the resistance rules. `saturation_current` with `forward_emission`, and `saturation_current` with `reverse_emission`, follow `finite_junction`, which names the emission of the junction that breaks it, as [netlist.md](netlist.md#npn-and-pnp-transistors) says. An `nmos` `transconductance_parameter` and its three capacitances follow the resistance rules. Its `threshold_voltage` only has to be finite and in volts. Its `channel_length_modulation` must be at least 0, which breaks `non_negative`, and a wrong unit on that field reports `netlist.unit` alone. [netlist.md](netlist.md#n-channel-mosfet) says why. A rating must be positive, with field `ratings.current` or `ratings.power`. Capacitance and a switch's two resistances follow the resistance rules. Every relay field follows the resistance rules, and a `drop_out_current` at or above `pull_in_current` breaks `hysteresis` on `drop_out_current`. `bounce.window` and `bounce.max_bounces` must be positive, and `bounce.window` must be at least `2 * max_bounces * 2000`, which breaks `bounce_room`. An NE555's resistances follow the resistance rules, with dotted fields such as `discharge.on_resistance`. Its `reset_threshold` must be positive, and `output.high_drop` and `output.low_drop` must not be negative. An ATmega328P's output stage follows the NE555's, and its two pull-ups the resistance rules. `clock_hz` must be positive. Each of `input_threshold.low`, `input_threshold.high`, `reset_threshold.low` and `reset_threshold.high` must be inside (0, 1), which breaks `fraction`, and a `low` above its `high` breaks `above_high` on `low`. A fuse byte that selects a clock other than a crystal or ceramic resonator, or programs `CKOUT`, is `unsupported` on `fuses.low`. One that programs `DWEN` or `BOOTRST` is `unsupported` on `fuses.high`, and one with a reserved `BODLEVEL` is `unsupported` on `fuses.extended`. [netlist.md](netlist.md#atmega328p) says why. `value` is then the byte. Its `leakage_resistance` follows the resistance rules. A number outside the f64 range is `netlist.syntax`, and so is a non-integer `bounce` value, a non-integer or oversized `clock_hz`, and a fuse byte that is not an integer from 0 to 255.

## netlist.firmware

Severity `error`. Refs is the part. `t_ps` is `null`.

The message is `{ref} firmware line {line} {what}`, or `{ref} firmware {what}` when `line` is `null`. `{what}` says the rule.

`data` holds `field`, `line`, and `rule`. `field` is `firmware`. `line` is the 1-based line of the Intel HEX text at fault, or `null` when no one line is. `rule` is one of these.

- `record`: the line is not an Intel HEX record. It has no `:`, a digit that is not hexadecimal, an odd number of digits, or a length that does not match its byte count, or an address record is not the length its type takes, or an end-of-file record has data.
- `checksum`: the record's bytes do not sum to 0.
- `record_type`: the type is not 00 to 05.
- `after_end`: a record follows the end-of-file record.
- `no_end`: the text has no end-of-file record. `line` is `null`.
- `past_flash`: a data byte lands at or past byte 32768, the end of the ATmega328P's flash.
- `overlap`: a data byte lands where an earlier record placed one. `line` is the later record's.

Parse stops at the first fault, so a part has at most one. [netlist.md](netlist.md#atmega328p) says how the text becomes flash.

## netlist.same_net

Severity `error`. Refs are two pins of one part, in order: the pin at fault, then the pin its kind ties it to, for example `#u1.pin.GND_22` and `#u1.pin.GND_8`. `t_ps` is `null`.

The message is:

```text
pin #u1.pin.GND_22 is on `agnd`; it must be on `gnd`, as #u1.pin.GND_8 is
```

`data` holds `expected` and `found`, both strings. `found` is the net the first pin names, and `expected` the net of the second. An ATmega328P's `GND_22` must be on the net of its `GND_8`, as [netlist.md](netlist.md#atmega328p) says.

## netlist.ground

Severity `error`. No refs. `t_ps` is `null`.

The message is:

```text
ground `{net}` is not in `nets`
```

`data` holds `net`.

## netlist.analysis

Severity `error`. No refs. `t_ps` is `null`.

The message is `analysis {field} is {value}; {requirement}`, for example:

```text
analysis max_step_ps is 999; it must be at least 1000
```

`data` holds `field`, `rule`, and `value`. `field` is `stop_ps`, `max_step_ps` or `grid_ps`. `rule` is one of these.

- `positive`: `stop_ps` is 0.
- `step_under_1ns`: `max_step_ps` is under 1000, the first step of a segment.
- `step_over_stop`: `max_step_ps` is greater than `stop_ps`.
- `grid_under_1ns`: `grid_ps` is under 1000.
- `grid_over_stop`: `grid_ps` is greater than `stop_ps`.

One analysis can break more than one rule.

## netlist.schedule

Severity `error`. Refs is the part. `t_ps` is `null`.

The message names the event by its index in `field`. `field` is `events` for a switch, `voltage.events` or `current.events` for a `steps` or `serial` source, and `position.events` for a potentiometer. Only the message reads `field`. `data` holds `event` and `rule`.

A switch that is too close says, for example:

```text
#sw1 events[1] is less than 2000 ps after the previous event and its bounce window
```

A source level that is too close says `#v1 voltage.events[1] is less than 2000 ps after the previous event`. A potentiometer position uses that sentence on `position.events`. A level that does not change says `#v1 voltage.events[1] does not change the level`. A switch that does not change says the contact does not change. A `serial` event with no bytes uses that level sentence. `at_zero` and `not_increasing` use the same sentence for every field. A switch `overflow` says the bounce window ends past the largest time. A `serial` `overflow` says the event ends past the largest time, for example `#s1 voltage.events[0] ends past the largest time`. A `serial` `overlap` says the event starts before the previous event ends, and names that end, for example `#s1 voltage.events[1] starts before the previous event ends at 21000 ps`. `data` does not hold the end.

`rule` is `at_zero`, `same_state`, `not_increasing`, `too_close`, `overflow`, or `overlap`, as [netlist.md](netlist.md#switch) defines them for a switch and [netlist.md](netlist.md#source-waveforms) defines them for a level and a serial event. Each broken event is one diagnostic. After `overflow`, later events of that part are not checked. An event whose end does not fit is `overflow` even when it also starts before the previous event ends. An event that is not after the previous one says `#sw1 events[1] is not after the previous event`.

## netlist.waveform

Severity `error`. Refs is the source. `t_ps` is `null`.

The message is `{ref} {field} is {value}; {requirement}`, for example:

```text
#i1 current.width_ps is 1500; two jumps must be at least 2000 ps apart
```

`data` holds `field`, `rule`, and `value`. `field` is the time's dotted field, such as `voltage.at_ps` or `current.period_ps`. `rule` is `at_zero`, `too_close`, `overflow`, or `equal`, as [netlist.md](netlist.md#source-waveforms) defines them. Each broken rule is one diagnostic, and one waveform can break more than one. An `at_zero` message ends `a jump must come after t = 0`, and an `overflow` message ends `the first fall is past the largest time`.

`equal` says the two levels are equal, for example `#s1 voltage.mark and voltage.space are equal`. Its `field` is `voltage.space` or `current.space`, and `value` is 0. A `baud` above 500000000 uses `too_close` on field `voltage.events[k].baud` or `current.events[k].baud`, and `value` is that baud.

## net.floating

Severity `warning`. Refs is `#net.<name>`. `t_ps` is `null`.

The message is `{ref} has no DC path to ground`.

`data` is `{}`.

A DC path is a chain of resistors, potentiometers, voltage sources, diodes, LEDs, transistors, inductors, relays, and switches from the net to ground. A resistor, voltage source, diode, LED, inductor, or switch joins its two pins. An inductor joins them through its series resistance, because its inductance is a short at DC. A relay joins `coil_a` to `coil_b` through its coil, and `com` to `no` and `com` to `nc` in either state, so a net on a contact's open side is not floating. A potentiometer joins `a` to `wiper` and `wiper` to `b`. An `npn` or a `pnp` joins `collector` to `base` and `base` to `emitter`, through its junctions. An `nmos` joins `drain` to `source`. Its gate joins nothing, so a net reached only through gates and capacitors is `net.floating`. A switch joins its pins open or closed. An NE555 joins VCC to CONT, CONT to GND, VCC to GND, OUT to VCC, OUT to GND, and DISCH to GND, in either latch state, and joins nothing to TRIG, THRES, or RESET. An ATmega328P joins each port pin, `PC6` included, to VCC and to `GND_8`, through the leakage it stamps in every drive state. Its `GND_22` is on the net of its `GND_8`. It joins nothing to `PB6`, `PB7`, `AREF`, or `AVCC`, as [netlist.md](netlist.md#atmega328p) explains. A capacitor with `"initial": null` does not join its pins, because it is open at DC. Neither does a current source. A capacitor with an `initial` voltage joins them, because the operating point holds it. gmin, 1e-12 S from every node to ground, is separate from that chain.

`net.floating` is a static check on the netlist's topology. It never depends on a solve, so its `t_ps` is always `null`. A run that reads an input nothing drives will report that under a code of its own.

A resistor from `vin` to `lonely`, with a voltage source from `vin` to `gnd`, puts `lonely` on a path to ground. Two LEDs in series from `vled` to `gnd`, with a resistor from `vin` to `vled`, put `mid` on a path to ground. A net that no part names is floating. Two nets whose only part is a resistor between them are floating. The operating point is still written, and `sim run` exits 0.

## solver.singular

Severity `error`. Refs is the unknown the zero pivot names, a net or a part. `t_ps` is 0 for the operating point. In a transient it is the end of the step that failed, the probe's time for a probe locating a chip event, or the restart's time for the restart solve after a switch edge, a source jump or a chip drive change.

The message is `singular matrix at {ref}`.

`data` is `{}`.

Two voltage sources in parallel, set to different voltages, are singular. With `v1` at 5 V and `v2` at 3 V, both from `vin` to `gnd`, the ref is `#v2`.

## solver.nonfinite

Severity `error`. Refs is the unknown or the part. `t_ps` is 0 for the operating point. In a transient it is the end of the step that failed, or the restart's time for the restart solve after a switch edge, a source jump or a chip drive change.

The message is `non-finite number at {ref}`.

`data` is `{}`.

In a circuit with no diode or LED, a solution component that is not finite names that unknown. With one, the fallback ladder retries instead, and a ladder that runs out reports `solver.convergence`. An observed `ib`, `ic`, `id`, `i`, `v`, or `p` that is not finite names the part. A 1e200 V source across 1 ohm overflows `p` on the resistor.

## solver.too_large

Severity `error`. No refs. `t_ps` is 0 for the operating point.

The message is `{unknowns} unknowns exceeds the dense solver limit of {limit}`.

`data` holds `limit` and `unknowns`. `limit` is 4096. The solver checks the count before it allocates the dense matrix, at t = 0, so `observations.json` is `{}`.

## solver.convergence

Severity `error`. Refs is the junction part that failed worst in the last Newton iteration, or else the unknown that moved most against its tolerance. A rung whose solution is not finite names that unknown. `t_ps` is 0 for the operating point, and the restart's time for the restart solve after a switch edge, a source jump or a chip drive change.

The code comes from one of four solves, told apart by `t_ps`, `step_ps` and the message.

- The operating point. `t_ps` is 0, and `data` has no `step_ps`.
- The restart solve after a switch edge, a source jump or a chip drive change. `t_ps` is the restart's time, after 0, and `data` has no `step_ps`. The trace ends with the row at that time.
- A time step. `data` has `step_ps`, and `t_ps` is the last accepted time, the trace's `end_ps`.
- A probe locating a chip event. `data` has `step_ps`, and `t_ps` is the probe's time, after the trace's `end_ps`.

For the operating point the message is:

```text
Newton did not converge at {ref} after {iterations} iterations; source stepping reached {source_scale}
```

`data` holds `gmin`, `iterations`, and `source_scale`. Nothing solved, so `observations.json` is `{}`.

The operating point starts with Newton at nominal conditions. If that fails, gmin stepping adds a conductance to ground from every node but a bipolar transistor's internal ones and the internal node of a diode or an LED, from 1e-2 S down by decades, and ends at nominal. An `nmos` has no internal node, so the stepping reaches its drain, gate and source. If that fails, source stepping scales every source from 0 up to 1. This code means all three failed. `iterations` counts Newton iterations over all of them. `gmin` is the smallest added conductance that converged, in S, or `null`. `source_scale` is the largest scale that converged, or 0.

The restart solve runs the same ladder, with every capacitor held at its voltage, every inductor held at its current, and the new contacts, source levels and chip states in force. A restart solve follows each switch edge and source jump, and each chip event whose new state stamps differently, at its time. A flip that changes no drive takes none. Its `data` is the same as the operating point's. Its message names the restart and its time:

```text
Newton did not converge at {ref} after {iterations} iterations in the restart solve at {t_ps} ps; source stepping reached {source_scale}
```

A fallback that succeeds is not reported. Its answer converged at nominal conditions, the same as plain Newton's. A singular matrix at any step ends the run as `solver.singular`.

In a transient, a step whose Newton solve stalls is halved and retried, down to 1 ps. When the 1 ps step stalls too, the code is reported with `t_ps` at the last accepted time, which is also the trace's `end_ps`. The message is:

```text
Newton did not converge at {ref} after {iterations} iterations; the time step halved to {step_ps} ps
```

`data` adds `step_ps`, the last step tried. `iterations` counts Newton iterations over every attempt that stalled since the last accepted step, from the first stall down to 1 ps. An attempt rejected for its truncation error converged, so it does not count. `gmin` is `null`, and `source_scale` is 1. The operating point's `data` has no `step_ps` key.

A probe locating a chip event re-solves a step that already converged, so it is not halved. When its midpoint probe stalls, `t_ps` is the probe's time, after the trace's `end_ps`. The message is:

```text
Newton did not converge at {ref} after {iterations} iterations in the probe at {t_ps} ps locating a chip event
```

`data` adds `step_ps`, the probe's step from the last accepted time. `iterations` is that probe's.

## solver.step_limit

Severity `error`. No refs. `t_ps` is the last accepted time, which is the trace's `end_ps`.

The message is:

```text
the run accepted {limit} steps, its limit; dense switch or source schedules cost steps, not rows
```

`data` holds `limit`. The limit is 100000000 accepted steps. The operating point at t = 0 does not count. The run keeps the observations and the last row up to that time.

A grid that fits in the trace can still ask for more steps than the limit. Edges and jumps are landings, not rows, so a dense switch or source schedule reaches the limit on steps.

## chip.unsettled

Severity `error`. Refs are the chips whose state changed on the last solve, in part order. `t_ps` is 0 for the operating point, or the time of the restart solve.

The message is `the chips kept changing state for {rounds} solves at {t_ps} ps`.

`data` holds `rounds`, which is 8. At one time the run solves, lets every chip react to the levels on that solve, and solves again while any state changed. A chip whose output drives its own inputs with nothing to slow it never agrees with its levels. An NE555 with OUT wired to TRIG and THRES is one: low sets the latch and high resets it. So are two NE555s, each OUT on the other's TRIG, that reset at one time. So is a relay whose `nc` feeds its own coil, when that coil is powered at t = 0. Open, the coil current is above pull-in. Closed, it is below drop-out. When the coil is powered later, it holds its current across each move and the contact buzzes. At t = 0 `observations.json` is `{}`.

## trace.too_large

Severity `error`. No refs. For a `sim` build, `t_ps` is `null`.

The message is `the grid gives {rows} rows; the trace holds {limit} rows of {columns} columns; raise grid_ps or shorten stop_ps`.

`data` holds `columns`, `limit` and `rows`. `rows` is `ceil(stop_ps / grid_ps) + 1`, the rows the grid gives. `limit` is the row cap, `min(1000000, 67108864 / columns)`. 67108864 is 2^26, so the stored f64 values of `analog.f64` stay within 512 MiB. `envelope.f64` adds at most twice that again, and writing the folder keeps a second copy of both. `columns` is the number of trace columns.

The run checks the count before it solves anything, so a refused run writes `{}` to `observations.json` and no `traces/`. The rows depend only on `stop_ps` and `grid_ps`. Edges, jumps, chip events and the error control add steps, not rows, so a `sim` build that passes the check never runs out of rows. The test-only step-rows mode can also stop mid-run with `t_ps` the step that did not fit, keeping its rows.

## trace.row_interpolated

Severity `warning`. No refs. `t_ps` is the row's time.

The message is `the row at {t_ps} ps did not solve; it is the line between the steps at {from_ps} ps and {to_ps} ps`.

`data` holds `from_ps` and `to_ps`, the accepted steps on either side of the row. A row no step lands on is a probe from the last accepted step, as [engine.md](engine.md#rows) says. When its solve fails, it is solved once more with Newton started from the step after it. When that fails too, every column of the row is the straight line from the state the probe was solved from at `from_ps` to the step at `to_ps`, read at `t_ps`. After a restart at `from_ps` that state is the restart's solve, not the step's row. Every column of it takes the new source levels and contacts, a current source's and a switch's included. The run goes on. No step, stop, edge or other diagnostic changes, and a grid with no row between those two steps has no such warning.

## rating.current

Severity `warning`. Refs is the rated quantity, `#<part>.i`, `#<part>.ic` for a bipolar transistor, or `#<part>.id` for an `nmos`. `t_ps` is the first time over the rating, 0 for the operating point.

The message is:

```text
{ref} peaks at {peak} A, over its {limit} A rating
```

`data` holds `limit`, `over_ps`, `peak`, and `peak_ps`. `limit` is the rating. `peak` is the largest magnitude of the quantity. `peak_ps` is when it peaked. `over_ps` is `null` for an operating point, which has no duration.

A transient checks every accepted step, not only the rows. Each step's value holds until the next accepted step. `t_ps` is the first step over the rating, and `over_ps` adds the time to the next step for every step over it. The last step adds nothing. So `t_ps` and `peak_ps` can fall between rows. At a switch edge the solve at the edge holds the old contact. With 22 ohm in place of 220 ohm in `examples/button-led`, `#led1.i` is first over 1 ns after the press, and `over_ps` is exactly the time the contact was closed.

A quantity is over when its magnitude is above the rating. Equal is not over. `peak` is the largest magnitude. For an operating point it equals `abs(observations.op[ref])`. The operating point is still written, and `sim run` exits 0.

The 9 V LED example puts about 0.55 A through an LED rated 0.03 A.

## rating.power

Severity `warning`. Refs is the rated quantity, `#<part>.p`. `t_ps` is the first time over the rating, 0 for the operating point.

The message is:

```text
{ref} peaks at {peak} W, over its {limit} W rating
```

`data` is the same as `rating.current`.

The 9 V LED example reports `#led1.p` over its 0.075 W rating and `#r1.p` over its 0.25 W rating.
