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

`code`, the condition it names, the shape of `refs`, the meaning of `t_ps`, and the keys and types of `data` are stable. A condition never moves to another code. `message` is for people and its text can change, so match on `code` and `data`. New codes, severities and `data` keys can appear, and a reader accepts them.

The run sorts diagnostics by `t_ps`, then `code`, then `refs`, then the JSON bytes of the whole object. A `null` time sorts before a number, so every input finding precedes a solver finding.

A session's `diagnostics()` returns the warnings found so far, and once the session ends they are that list's. While it runs, a `Pulses` record still in flight is not yet counted, as [engine.md](engine.md#sessions) says. The folder's warnings are `net.floating`, `trace.row_interpolated`, `rating.current`, `rating.power`, `rating.voltage`, `chip.protocol`, `chip.link_miswired`, `chip.unmodeled_register` and `chip.restart`. `diagnostics()` returns all but `trace.row_interpolated`. It leaves out `trace.row_interpolated`, because that warning names a grid row whose probe failed twice, and only a recorded trace defines those rows. An open session has no grid. The accepted steps do not depend on the grid, and neither do the warnings `diagnostics()` returns. Every other code on this page is an error. The error that ends a run is `fault` on the session, and `diagnostics()` does not return it. [engine.md](engine.md#sessions) says when the samples are taken.

A run whose diagnostics are all warnings still writes the operating point. A run with an error writes the observations it reached, as [run-folder.md](run-folder.md#observationsjson) says. That is `{}` when nothing solved, as at t = 0. A transient keeps its grid rows before the error, and its last accepted step is the last row. Its observations end there. A warning found before that error is kept, including a rating sampled on any accepted step before it.

Stdout of `orbe-run run` lists the same objects in the same order.

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

`data` holds `expected`, `field`, and `found`. Resistance must be `ohm`. Voltage must be `V`. Current must be `A`. A source waveform's levels take the source's unit, with dotted fields such as `voltage.final` and `current.pulsed`. Capacitance must be `F`. A switch's `on_resistance` and `off_resistance` must be `ohm`, `bounce.window` must be `ps`, and `bounce.max_bounces` must be `1`. The diode, LED, zener and linear regulator fields, the `ratings.current` and `ratings.power` fields, and an `atmega328p`'s five `ratings` fields take the units in [netlist.md](netlist.md).

## netlist.value

Severity `error`. Refs is the part. `t_ps` is `null`.

The message for `positive` is `{ref} {field} is {value}; it must be positive`.

The message for `finite_reciprocal` is `{ref} {field} is {value}; its reciprocal must be finite`.

The message for `finite_junction` on a diode, an LED, or a transistor is `{ref} {field} is {value}; n*VT must be normal, and vcrit must be finite and positive`. On a zener, the `zener_impedance` message is `{ref} {field} is {value}; nbvt must be normal`. The `zener_voltage` message is `{ref} {field} is {value}; Isb must be normal and positive, and vcrit must be finite and positive`.

The message for `bounce_room` is `{ref} {field} is {value}; it must fit 2 * max_bounces edges 2000 ps apart`.

The message for `non_negative` is `{ref} {field} is {value}; it must not be negative`.

The message for `fraction` is `{ref} {field} is {value}; it must be between 0 and 1`.

The message for `below_one` is `{ref} {field} is {value}; it must be at least 0 and below 1`.

The message for `above_high` is `{ref} {field} is {value}; it must not be above high`.

The message for `hysteresis` is `{ref} {field} is {value}; it must be below pull_in_current`.

The message for `above_resistance` is `{ref} {field} is {value}; it must exceed series_resistance + breakdown_resistance`.

The message for `above_drop` is `{ref} {field} is {value}; it must exceed zener_current * (series_resistance + breakdown_resistance)`.

The message for `unsupported` is `{ref} {field} is {byte}; the model does not run that mode`, with the byte in hex, such as `0x62`.

The message for `span` is `{ref} {field} is {value}; it must be from 0 to 4095`.

The message for `range` is `{ref} {field} is {value} {unit}; {quantity} must be from {min} to {max} {unit}`, with the quantity's row of the [world table](netlist.md#world), for example `#world.light value is 200000.0 lx; light must be from 0.0 to 100000.0 lx`. A B equation's `x` names the temperature row.

The message for `finite` is `{ref} {field} is {value}; it must be finite`.

The message for `distinct` is `{ref} {field} is {value}; it must differ from x0`.

`data` holds `field`, `rule`, and `value`. `rule` is `positive`, `finite_reciprocal`, `finite_junction`, `bounce_room`, `non_negative`, `fraction`, `below_one`, `above_high`, `hysteresis`, `above_resistance`, `above_drop`, `unsupported`, `span`, `leds`, `range`, `finite`, or `distinct`. The message prints `value` in the same form as `data.value`, but for `unsupported`, which prints the byte, `leds`, which prints the count as an integer, and a value that is not finite, which `data.value` holds as `null` and the message prints as `NaN`, `inf` or `-inf`. Resistance must be positive, and its reciprocal must be finite. A value that breaks both reports `positive`. A potentiometer's `resistance` follows those rules. Its `position` must be inside (0, 1), which breaks `fraction`. A leg, `position * resistance` or `(1 - position) * resistance`, that breaks the resistance rules is `netlist.value` on field `resistance`, and `value` is that leg. A source's voltage or current, and each level of its waveform, only has to be finite and in its unit. The diode and LED fields follow the resistance rules. `saturation_current` and `emission_coefficient` also follow `finite_junction`, as [netlist.md](netlist.md) defines it. A zener's seven fields follow the resistance rules. `zener_impedance` must exceed `series_resistance + breakdown_resistance`, which breaks `above_resistance`. `zener_voltage` must exceed `zener_current` times that sum, which breaks `above_drop`. The derived breakdown junction follows `finite_junction`, naming `zener_impedance` or `zener_voltage` as [netlist.md](netlist.md#zener) says. An inequality that fails is reported alone, without `finite_junction`. A linear regulator's `output_voltage`, `output_resistance`, `dropout_voltage`, `dropout_resistance` and `current_limit` follow the resistance rules. Its `line_regulation` must be at least 0 and below 1, which breaks `below_one`. Its `quiescent_current` and `ground_current_ratio` must be at least 0, which breaks `non_negative`. Every `npn` and `pnp` field follows the resistance rules. `saturation_current` with `forward_emission`, and `saturation_current` with `reverse_emission`, follow `finite_junction`, which names the emission of the junction that breaks it, as [netlist.md](netlist.md#npn-and-pnp-transistors) says. An `nmos` `transconductance_parameter` and its three capacitances follow the resistance rules. Its `threshold_voltage` only has to be finite and in volts. Its `channel_length_modulation` must be at least 0, which breaks `non_negative`, and a wrong unit on that field reports `netlist.unit` alone. [netlist.md](netlist.md#n-channel-mosfet) says why. A rating must be positive, with field `ratings.current` or `ratings.power`, or on an `atmega328p` `ratings.pin_current`, `ratings.supply_current`, `ratings.rail_margin`, `ratings.reset_voltage` or `ratings.supply_voltage`. Capacitance and a switch's two resistances follow the resistance rules. Every relay field follows the resistance rules, and a `drop_out_current` at or above `pull_in_current` breaks `hysteresis` on `drop_out_current`. `bounce.window` and `bounce.max_bounces` must be positive, and `bounce.window` must be at least `2 * max_bounces * 2000`, which breaks `bounce_room`. An NE555's resistances follow the resistance rules, with dotted fields such as `discharge.on_resistance`. Its `reset_threshold` must be positive, and `output.high_drop` and `output.low_drop` must not be negative. An ATmega328P's output stage follows the NE555's, and its two pull-ups the resistance rules. `clock_hz` must be positive. Each of `input_threshold.low`, `input_threshold.high`, `reset_threshold.low` and `reset_threshold.high` must be inside (0, 1), which breaks `fraction`, and a `low` above its `high` breaks `above_high` on `low`. A fuse byte that selects a clock other than a crystal or ceramic resonator, or programs `CKOUT`, is `unsupported` on `fuses.low`. One that programs `DWEN` or `BOOTRST` is `unsupported` on `fuses.high`, and one with a reserved `BODLEVEL` is `unsupported` on `fuses.extended`. [netlist.md](netlist.md#atmega328p) says why. `value` is then the byte. Its `leakage_resistance` follows the resistance rules. A `cst816` coordinate above 4095 breaks `span` on `touches[n].x` or `touches[n].y`. A `ws2812` `leds` from 0 to 65535 outside 1 to 1024 breaks `leds`, and any other number is `netlist.syntax`. A world quantity's value breaks `range` on ref `#world.<quantity>`, with field `value`, `initial` or `events[k].level`. A world-driven field keeps its own rules at both ends of its quantity's range, on field `<field>.curve at <end> <unit>`, and breaks `finite` there when the curve is not finite, for example `#r1 resistance.curve at 125.0 degC is -1250.0; it must be positive`. A `power` curve's `x` follows the resistance rules' `positive`. A `beta` curve's `x` outside the temperature range breaks `range`. A `linear` curve whose `x1` equals its `x0` breaks `distinct` on `<field>.curve.x1`. A number outside the f64 range is `netlist.syntax`, and so is a non-integer `bounce` value, a non-integer or oversized `clock_hz`, and a fuse byte that is not an integer from 0 to 255.

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

An `esp32s3` part's `firmware.elf` blob has three rules, each with `line` `null`. `field` is still `firmware`.

- `elf`: the blob is not a 32-bit little-endian version 1 Xtensa ELF executable with a loadable segment, or a header, a segment or the symbol table runs past the file. A program header table must be at a nonzero offset with 32-byte entries and a count below `0xffff`. A section table with entries must be at a nonzero offset with 40-byte entries.
- `unmapped`: a loadable segment lies outside the chip's SRAM windows, or the entry lies in no segment on the instruction bus.
- `overlap`: two loadable segments reach one byte of SRAM, through one window or both. The message is `{ref} firmware loads two segments onto one byte of memory`.

An `esp32s3` part's `firmware.flash` and `rom` have these, each with `line` `null` and `field` `firmware`.

- `region`: a region lies past the end of the 16 MB flash, or on a byte an earlier region placed.
- `image`: the flash has no app ESP-IDF's bootloader would start. It has no partition table at `0x8000`, no factory or `ota_0` app partition, a partition past the flash's end, or otadata that is not blank, or the app partition holds no ESP32-S3 app image whose checksum, and hash when the header names one, hold. A segment in a cache window whose address and flash offset differ modulo the 64 KB MMU page is refused too, as IDF's bootloader refuses it.
- `unmapped`: an app segment lies outside the cache windows, the SRAM windows and RTC memory, or the entry lies in no segment in the instruction-bus SRAM window.
- `rom`: `rom` is not the revision 0 ROM the model runs, or is absent beside `firmware.flash`, or present beside `firmware.elf`. The message names the ROM's SHA-256 and length.

Parse stops at the first fault, so a part has at most one. [netlist.md](netlist.md#atmega328p) says how the text becomes flash, and [netlist.md](netlist.md#esp32-s3) how an ELF loads and how the flash finds its app.

## netlist.blob

Severity `error`. Refs is the part. `t_ps` is `null`.

A blob the part names is missing or is not the blob it names, as [netlist.md](netlist.md#blobs) says. The message names the part, the role and the hash. For a missing blob it is `{ref} {role} blob {sha256} is not in the blobs folder beside the netlist or in any --blobs folder`. For another hash it is `{ref} {role} blob {sha256}: no file under that name in the blobs folder beside the netlist or in a --blobs folder has that SHA-256`. For another length it is `{ref} {role} blob {sha256} in the blobs folder beside the netlist or a --blobs folder has {found} bytes; the netlist says {bytes}`. Those are the messages of `orbe-run run`. The wasm package takes a blob list, so there a missing blob is `{ref} {role} blob {sha256} is not in the blob list` and another length is `{ref} {role} blob {sha256} in the blob list has {found} bytes; the netlist says {bytes}`.

`data` holds `bytes`, `found`, `role`, `rule` and `sha256`. `bytes` is the netlist's length. `found` is the length of the file the run read under that name, or `null` when there is none. When no file under that name has the right hash, it is the first file found, in the search order of [netlist.md](netlist.md#blobs). `role` is `elf`. `sha256` is the hash the netlist names. `rule` is one of these.

- `missing`: no folder has a file under that name.
- `hash`: every file under that name hashes to another digest.
- `length`: the file's length is not `bytes`.

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

## netlist.world

Severity `error`. Refs is the part whose field reads the quantity, or empty for a name in the netlist's `world`. `t_ps` is `null`.

`data` holds `field`, `name`, and `rule`. `name` is the quantity's name as the netlist wrote it. `rule` is one of these.

- `unknown`: the name is not a row of the [world table](netlist.md#world). The message is ``{field} names `{name}`, which is not a world quantity; the quantities are temperature, humidity, light``, with the ref before it for a part's field, for example ``#r1 resistance.quantity names `lite`, which is not a world quantity; the quantities are temperature, humidity, light``. `field` is `world` for a name in `world` and `<field>.quantity` for a part's.
- `undeclared`: a part's field reads a quantity the netlist's `world` does not declare. The message is ``{ref} {field} reads {name}, which the netlist's `world` does not declare``. `field` is `<field>.quantity`, or `kind` when the part's device reads the quantity, as a `dht22` reads `temperature` and `humidity`.
- `temperature`: a `beta` curve reads a quantity other than `temperature`. The message is `{ref} {field} is a B equation, which reads temperature, not {name}`. `field` is `<field>.curve.kind`.
- `non_negative`: a `power` curve reads a quantity whose range goes below 0, which is `temperature`. The message is `{ref} {field} is a power curve, which reads a quantity that does not go below 0, not {name}`. `field` is `<field>.curve.kind`.

A world quantity's schedule follows [netlist.schedule](#netlistschedule), with ref `#world.<quantity>` and field `events`, and a level in another unit is [netlist.unit](#netlistunit) on that ref. A quantity whose value or schedule is rejected is still declared, so a part that reads it reports no `undeclared`.

## netlist.schedule

Severity `error`. Refs is the part. `t_ps` is `null`.

The message names the event by its index in `field`. `field` is `events` for a switch, `voltage.events` or `current.events` for a `steps` or `serial` source, `position.events` for a potentiometer, `events` on `#world.<quantity>` for a world quantity, `touches` for a `cst816`, and `console` for an `esp32s3`. Only the message reads `field`. `data` holds `event` and `rule`.

A switch that is too close says, for example:

```text
#sw1 events[1] is less than 2000 ps after the previous event and its bounce window
```

A source level that is too close says `#v1 voltage.events[1] is less than 2000 ps after the previous event`. A potentiometer position uses that sentence on `position.events`, and a world quantity on `events`, for example `#world.light events[3] is less than 2000 ps after the previous event`. A `cst816` touch uses it on `touches`, for example `#u2 touches[1] is less than 2000 ps after the previous event`. A level that does not change says `#v1 voltage.events[1] does not change the level`. A switch that does not change says the contact does not change. A `serial` event with no bytes uses that level sentence, and so does an `esp32s3` `console` event with no bytes. `at_zero` and `not_increasing` use the same sentence for every field. A switch `overflow` says the bounce window ends past the largest time. A `serial` `overflow` says the event ends past the largest time, for example `#s1 voltage.events[0] ends past the largest time`. A `serial` `overlap` says the event starts before the previous event ends, and names that end, for example `#s1 voltage.events[1] starts before the previous event ends at 21000 ps`. `data` does not hold the end.

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

A DC path is a chain of resistors, potentiometers, voltage sources, diodes, LEDs, zeners, linear regulators, transistors, inductors, relays, and switches from the net to ground. A resistor, voltage source, diode, LED, zener, inductor, or switch joins its two pins. A linear regulator joins `in` to `gnd` and `out` to `gnd`. An inductor joins them through its series resistance, because its inductance is a short at DC. A relay joins `coil_a` to `coil_b` through its coil, and `com` to `no` and `com` to `nc` in either state, so a net on a contact's open side is not floating. A potentiometer joins `a` to `wiper` and `wiper` to `b`. An `npn` or a `pnp` joins `collector` to `base` and `base` to `emitter`, through its junctions. An `nmos` joins `drain` to `source`. Its gate joins nothing, so a net reached only through gates and capacitors is `net.floating`. A switch joins its pins open or closed. An NE555 joins VCC to CONT, CONT to GND, VCC to GND, OUT to VCC, OUT to GND, and DISCH to GND, in either latch state, and joins nothing to TRIG, THRES, or RESET. An ATmega328P joins each port pin, `PC6` included, to VCC and to `GND_8`, through the leakage it stamps in every drive state. Its `GND_22` is on the net of its `GND_8`. It joins nothing to `PB6`, `PB7`, `AREF`, or `AVCC`, as [netlist.md](netlist.md#atmega328p) explains. An SPI replay joins `CS`, `SCLK` and `D0` to `D3` each to `VCC` and to `GND`, through the stage it drives. A capacitor with `"initial": null` does not join its pins, because it is open at DC. Neither does a current source. A capacitor with an `initial` voltage joins them, because the operating point holds it. gmin, 1e-12 S from every node to ground, is separate from that chain.

`net.floating` is a static check on the netlist's topology. It never depends on a solve, so its `t_ps` is always `null`. A run that reads an input nothing drives will report that under a code of its own.

A resistor from `vin` to `lonely`, with a voltage source from `vin` to `gnd`, puts `lonely` on a path to ground. Two LEDs in series from `vled` to `gnd`, with a resistor from `vin` to `vled`, put `mid` on a path to ground. A net that no part names is floating. Two nets whose only part is a resistor between them are floating. The operating point is still written, and `orbe-run run` exits 0.

## net.short

Severity `error`. Refs are the voltage source, then `#net.<name>`, the net both its pins are on. `t_ps` is `null`.

The message is:

```text
{part} holds `p` and `n` apart, but both are on {net}, so the circuit shorts it
```

`data` is `{}`.

Parse checks it, so the netlist is refused and nothing solves. An ideal source across one net has a branch row of zeros, which the solve would report as `solver.singular` at the source. A wire from a board's 5V straight to its GND does this on a board whose USB source feeds 5V with nothing in between. A source of 0 V across one net is the same short.


Severity `error`. Refs is the unknown the zero pivot names, a net or a part. `t_ps` is 0 for the operating point. In a transient it is the end of the step that failed, the probe's time for a probe locating a chip event, or the restart's time for the restart solve after a switch edge, a source jump or a chip drive change.

The message is `singular matrix at {ref}`.

`data` is `{}`.

A voltage source whose two pins are on one net never gets here. Parse refuses it as [net.short](#netshort). Two voltage sources in parallel, set to different voltages, are singular. With `v1` at 5 V and `v2` at 3 V, both from `vin` to `gnd`, the ref is `#v2`.

## solver.nonfinite

Severity `error`. Refs is the unknown or the part. `t_ps` is 0 for the operating point. In a transient it is the end of the step that failed, or the restart's time for the restart solve after a switch edge, a source jump or a chip drive change.

The message is `non-finite number at {ref}`.

`data` is `{}`.

In a circuit with no diode, LED or zener, a solution component that is not finite names that unknown. With one, the fallback ladder retries instead, and a ladder that runs out reports `solver.convergence`. An observed `ib`, `ic`, `id`, `iin`, `iout`, `i`, `v`, or `p` that is not finite names the part. A 1e200 V source across 1 ohm overflows `p` on the resistor.

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

The operating point starts with Newton at nominal conditions. If that fails, gmin stepping adds a conductance to ground from every node but a bipolar transistor's internal ones and the internal nodes of a diode, an LED or a zener, from 1e-2 S down by decades, and ends at nominal. An `nmos` has no internal node, so the stepping reaches its drain, gate and source. If that fails, source stepping scales every source from 0 up to 1. This code means all three failed. `iterations` counts Newton iterations over all of them. `gmin` is the smallest added conductance that converged, in S, or `null`. `source_scale` is the largest scale that converged, or 0.

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

## chip.protocol

Severity `warning`. Refs are the chip. `t_ps` is the first time that rule fired. A dropped SPI transaction uses the end of that transaction.

The message for a dropped transfer is `{part} dropped a transfer that broke rule {rule} at {t_ps} ps, the first time`.

`data` holds `rule`. A chip that takes `Spi` records drops one that breaks its protocol, and the run reports each rule once per part, at its first time. An `st77916` has two rules, which [netlist.md](netlist.md#st77916) describes. A `cst816` has one more. Rule `sda_while_scl_high` means an `SDA` change came due while `SCL` was high, so the target left `SDA` where it was and went idle. A release it refused waits for the next `SCL` fall. A release while the part is in reset is not this rule. Its message is `{part} left SDA unmoved because SCL was high at {t_ps} ps, breaking rule sda_while_scl_high, the first time`. [engine.md](engine.md#i2c-targets) describes the hold. A `ws2812` has one more. Rule `bit` means a high, or a low after a high that was in a window, fell outside the WS2812E-1313 times, so that bit was dropped. Its message is `{part} dropped a bit that broke rule bit at {t_ps} ps, the first time`. [engine.md](engine.md#records) has the windows. A `dht22` has one more. Rule `interval` means a start came inside the sensor's 2 s warm-up, or within 2 s of the last start it answered, so the sensor did not answer it. Its message is `{part} did not answer a read that broke rule interval at {t_ps} ps, the first time`. [netlist.md](netlist.md#dht22) has the times.

| `rule` | What broke |
| --- | --- |
| `command` | Its command word is cut short by a release of `CS` or followed by phases, its opcode is not the one that command takes, its data lanes after the word do not match that opcode, a transaction with phases comes while a command is open, or its register command is short of bytes or out of range. A continuation whose data lanes do not match drops that transaction's data, and a release still closes the open command. Opcode 0x02 writes a register on one lane. Opcode 0x32 writes pixels on four lanes, and only `RAMWR` and `RAMWRC` take it. |
| `pixel_format` | It starts a pixel write while the pixel format is not 16 bits a pixel. |
| `sda_while_scl_high` | An I2C target's `SDA` change came due while `SCL` was high. The target goes idle. A refused release waits for the next `SCL` fall. A release in reset is not this rule. |
| `bit` | A `ws2812` high was in neither the WS2812E-1313 T0H window nor its T1H window. That bit is dropped, and its time is the fall that ended the high. The chain reports the first one. |
| `interval` | A `dht22` start came within 2 s of power-on or of the last start it answered. The sensor does not answer it, and its time is the end of the start's low. |

## chip.link_miswired

Severity `warning`. Refs are the chip that dropped a transaction. `t_ps` is the end of the first transaction it dropped.

The message is `{part} dropped an SPI transaction from #{sender} whose data lanes are not on its own data pins at {t_ps} ps, the first time`.

`data` holds `sender`, the part name of the chip that sent it. A transaction goes to every chip whose `CS` and `SCLK` are on the sender's. A lane it uses, up to the most lanes any of its phases or its data take, must be on the receiver's pin of the same number, `D0` to `D3`. Otherwise the receiver drops it, and the run reports the first such drop once per part.

## chip.unsupported_instruction

Severity `error`. Refs are the chip. `t_ps` is the time the core reached the instruction. It ends the run.

The message is `{part} reached {what} at {pc}, which the model does not run, at {t_ps} ps`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `op`, `pc`, `reason` and `symbol`. `op` is the op's name. `pc` is its address. `symbol` is the ELF symbol at or below the PC, written `name+0x1c` past its start, or `null`. When the APP CPU ran the code, the message has ` on the APP CPU` after the PC and its symbol, and `data` also holds `core`, `"app"`. Every `esp32s3` diagnostic with a `pc` writes the core this way. `reason` is one of these.

- `op`: the core does not model the op.
- `unaligned`: the op is an `l32ai`, an `s32ri` or an `s32c1i` at an address that is not a multiple of 4, or another load or store whose access is unaligned and one of whose parts faults.
- `litbase`: an `l32r` while `LITBASE` is enabled.
- `loop_end`: the op is the last of a zero-overhead loop and loops back, and it is a call, `isync`, `waiti` or `rsr.lcount`, which the ISA rules out there.

## chip.unsupported_setting

Severity `error`. Refs are the chip. `t_ps` is the time of the store that started the transfer or set the setting. It ends the run.

The message is `{part} started a {peripheral} transfer with setting {setting} at {pc}, which the model does not run, at {t_ps} ps` for a transfer, `{part} enabled a {peripheral} interrupt at {pc}, which the model never raises, at {t_ps} ps` for an interrupt, `{part} set {peripheral} {setting} at {pc}, which the model does not run, at {t_ps} ps` for a register setting, with `; ESP-IDF sets GDMA's reset when it restarts or panics, so the console holds any panic text` after `does not run` for `GDMA` `reset_while_sending`, and `{part} put the chip to sleep at {pc}, which the model does not run, at {t_ps} ps` for sleep, each with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `peripheral`, `pc`, `setting` and `symbol`. `pc` is the address of the store, and `symbol` is written as `chip.unsupported_instruction` writes it. For a transfer, `peripheral` is `SPI2`, and `setting` is the first of these the registers held when the store wrote `SPI_USR`: `slave`, `dma`, `dma_link`, `dma_descriptor`, `dma_length`, `lsb_first`, `qpi`, `octal`, `dtr`, `dummy`, `read`, `high_part`, `three_line`, `full_duplex`, `clock_mode`, `cs_disabled`, `data_length`, `clock_off` or `unsynced`. `setting` is `interrupt` when the store set an interrupt enable bit of a source the model never raises, with `peripheral` `SPI2`, `GDMA`, `TIMG0`, `TIMG1`, `USB_SERIAL_JTAG`, `SHA`, `LEDC` or `RMT`. It is `sleep`, with `peripheral` `RTC_CNTL`, when the store set `RTC_CNTL_SLEEP_EN`. These register settings end the run too:

- `memory_copy`, `write_back` and `restart`, with `peripheral` `GDMA`: the store set `GDMA_MEM_TRANS_EN`, `GDMA_OUT_AUTO_WRBACK`, `GDMA_INLINK_RESTART` or `GDMA_OUTLINK_RESTART`.
- `link_peripheral`, with `peripheral` `GDMA`: the store started an OUT link whose `GDMA_PERI_OUT_SEL` names neither SPI2 nor the SHA accelerator, or an IN link at a descriptor in SRAM whose `GDMA_PERI_IN_SEL` is not SPI2.
- `peri_sel_while_started`, with `peripheral` `GDMA`: the store changed a channel's `GDMA_PERI_OUT_SEL` or `GDMA_PERI_IN_SEL` while its link was started.
- `buffer_write`, with `peripheral` `GDMA`: the store reached a byte of a descriptor or a buffer that a running SPI2 transfer sends from. A byte that only shares a word with one does not count.
- `mode`, `dma_link`, `dma_descriptor` and `dma_length`, with `peripheral` `SHA`: the store started a hash in another mode than SHA-256, or a DMA hash whose GDMA channel or list SPI2 would refuse with that setting. A transfer setting `dma_link` also covers a start while GDMA's clock is off.
- `reset_while_sending` and `clock_off_while_sending`, with `peripheral` `SPI2` or `RMT`: the store set that peripheral's reset bit, or cleared its clock bit, while an SPI2 transfer or an RMT transmission ran. With `peripheral` `GDMA`, the store did that to GDMA's bits while an SPI2 transfer sent from it. ESP-IDF's restart and panic paths set GDMA's reset, `SYSTEM_DMA_RST`, in `esp_system_reset_modules_on_exit`, so an `esp_restart()` or a panic while SPI2 sends from GDMA ends the run here, at a PC in that function, in place of `chip.restart` or `chip.halted`. The console holds a panic's text, which ESP-IDF prints first.
- `clock_off` and `held_in_reset`, with `peripheral` `SPI2`, `LEDC`, `GDMA`, `SHA`, `I2C0` or `RMT`: a load or a store reached that peripheral, or `RMTMEM` for `RMT`, while its clock gate was clear, or while a reset bit held it. `pc` is that access's.
- `channel`, `dma`, `sync`, `rx`, `stop_while_sending` and `start_while_sending`, with `peripheral` `RMT`: a store to `CHnCONF0` for n from 1 to 3 sets `TX_START` or `IDLE_OUT_EN`, a store to `CHnCONF0` for n from 0 to 3 sets `DMA_ACCESS_EN`, a store sets `TX_SIM_EN`, a store to `CHmCONF1` for m from 4 to 7 sets `RX_EN`, or a store sets `TX_STOP` or `TX_START` on channel 0 while it sends. `TX_STOP` is named before `TX_START`.
- `fifo`, `power_down`, `clock_source`, `sclk_active`, `fractional_divider`, `mem_size`, `carrier`, `loop`, `no_wrap`, `idle_out` and `inverted`, with `peripheral` `RMT`: `TX_START` on channel 0 finds `APB_FIFO_MASK` clear, `MEM_FORCE_PD` set, `SCLK_SEL` other than 1, `SCLK_ACTIVE` clear, `SCLK_DIV_A` other than 0, `MEM_SIZE` other than 1, `CARRIER_EN` set, `TX_CONTI_MODE` set, `MEM_TX_WRAP_EN` clear, `IDLE_OUT_EN` clear, or `OUT_INV_SEL` set on a pad that carries signal 81. A store to `CHnDATA`, offsets `0x00` to `0x1c`, is `fifo` as well. The store writes nothing.
- `clock_change_while_sending` and `no_end`, with `peripheral` `RMT`: a store would change the APB rate while channel 0 sends, and it writes nothing, or a transmission reaches 65,536 entries with no end word. For `no_end`, `pc` is the instruction after the store that started the transmission.
- `app_cpu_without_reset`, with `peripheral` `SYSTEM`: a store made the APP CPU runnable and it has not been reset in this run, so the model cannot know its state. The run ends at a later store to `SYSTEM_CORE_1_CONTROL_0_REG` that does not set `RESETING`, or when 1,000 PRO CPU cycles pass with no reset. `pc` is that store, or the instruction the PRO CPU had reached at the bound, which need not be a store. The 1,000 cycles are the model's bound, not a chip figure. ESP-IDF's `cpu_utility_ll_enable_clock_and_reset_app_cpu` sets `CLKGATE_EN`, clears `RUNSTALL`, then sets and clears `RESETING`, and the APP CPU starts at that clear, as [esp32s3.md](esp32s3.md#the-two-cores) says.
- `pro_cpu_stall`, with `peripheral` `RTC_CNTL`: the store left `RTC_CNTL_SW_STALL_PROCPU_C0` at 2 and `RTC_CNTL_SW_STALL_PROCPU_C1` at `0x21`, which would stall the PRO CPU. The model does not stall it.
- `watchpoint`, with `peripheral` `CPU`: a load or a store reached a byte of a window `DBREAKAn` and `DBREAKCn` name. `pc` is that instruction, and the access does not complete. `DBREAKCn` bits 0 to 5 select which low address bits are compared, a set bit compared and a clear bit ignored. Bit 30 arms a load and bit 31 a store.
- `hpoint`, `fade` and `duty_fraction`, with `peripheral` `LEDC`: the store sent a channel's update with a nonzero `LEDC_HPOINT_LSCHn`, with `LEDC_DUTY_START_LSCHn` and a nonzero `LEDC_DUTY_SCALE_LSCHn`, or with a duty that has a fraction.
- `clock`, `tick_sel`, `clock_divider`, `duty_resolution`, `fractional_divider`, `duty_range` and `full_duty`, with `peripheral` `LEDC`: an update waits on a counting timer whose `LEDC_APB_CLK_SEL` is neither 1 nor 3, whose `LEDC_TICK_SEL_LSTIMERn` is set, whose divider is below 256, whose resolution is 0 or above 14, or whose period is not whole ticks of 480 MHz, or the update's duty is above that timer's steps, or is every step at 14 bits.
- `timer_change_while_running`, `pause` and `clock_change_while_running`, with `peripheral` `LEDC`: the store changed a counting timer's resolution, divider or tick source, paused it, or changed `LEDC_APB_CLK_SEL` or the APB clock while a timer counted it.
- `clock_off_while_running`, with `peripheral` `LEDC`: the store cleared LEDC's clock gate while a timer counted.
- `reset_while_running`, with `peripheral` `LEDC`: the store set LEDC's reset bit while a pin with a net carried a channel through the matrix, or while an update had yet to take effect or to drive its signal.
- `watchdog_cpu_reset`, `watchdog_interrupt` and `watchdog_stage`, with `peripheral` `TIMG0`, `TIMG1` or `RTC_CNTL`: the store left a running watchdog, at or before its first stage that resets, a CPU reset while its `PROCPU_RESET_EN` is clear, an interrupt of the RTC watchdog, or a stage field that names no action.
- `super_watchdog`, with `peripheral` `RTC_CNTL`: the store left both `SWD_AUTO_FEED_EN` and `SWD_DISABLE` clear in `RTC_CNTL_SWD_CONF_REG`.
- `pin_int_ena`, `pin_int_type` and `pin_wakeup`, with `peripheral` `GPIO`: the store set a `GPIO_PINn` word whose `INT_ENA` enables the NMI or an SDIO interrupt, whose `INT_TYPE` is the reserved 6 or 7, or whose `WAKEUP_ENABLE` is set.
- `port`, with `peripheral` `I2C1`: a store reached `0x6002_7000` to `0x6002_7fff`. One port is run.
- `slave_mode`, `push_pull`, `not_routed`, `lsb_first`, `arbitration`, `sample_scl_level`, `clock_off` and `timing`, with `peripheral` `I2C0`: `I2C_TRANS_START` left `MS_MODE` clear, left `SDA_FORCE_OUT` or `SCL_FORCE_OUT` clear or a routed pad without `PAD_DRIVER`, left signal 89 or 90 unrouted both ways, set `TX_LSB_FIRST` or `RX_LSB_FIRST`, set `ARBITRATION_EN`, set `SAMPLE_SCL_LEVEL`, left `SCLK_ACTIVE` clear, or copied a timing whose SDA hold is at least the SCL low period or whose SDA sample is at least `I2C_SCL_HIGH_PERIOD`. `I2C_CLK_EN` is kept and is not this `clock_off`.
- `rc_fast_clock`, `fractional_divider`, `nonfifo`, `fsm_reset`, `clear_bus`, `power_down` and `int_ena`, with `peripheral` `I2C0`: the store set `SCLK_SEL`, a nonzero `SCLK_DIV_A` or `SCLK_DIV_B`, `NONFIFO_EN`, `FSM_RST`, `SCL_RST_SLV_EN`, `SCL_PD_EN` or `SDA_PD_EN`, or an `INT_ENA` bit outside the NACK, timeout, transfer-complete, arbitration-lost and end-detect bits.
- `end_command` and `opcode`, with `peripheral` `I2C0`: a command is END, or is op 0, 5 or 7, or a read or a write whose byte count is 0, or the first command is neither a repeated START nor a STOP.
- `tx_underflow`, `tx_overflow`, `rx_overflow` and `rx_underflow`, with `peripheral` `I2C0`: a write command finds the TX FIFO short, a store to `I2C_DATA_REG` finds it full, a read command finds the RX FIFO full, or a load of `I2C_DATA_REG` finds it empty.
- `busy_write`, with `peripheral` `I2C0`: a store of `TRANS_START`, `CONF_UPGATE`, a command, a FIFO reset or a timing word came while a transfer ran.
- `timeout`, with `peripheral` `I2C0`: a wait for SCL to read high reached `2^I2C_TIME_OUT_VALUE` sclk cycles. The register header says APB cycles, so the sclk count is inferred from IDF's driver, as [esp32s3.md](esp32s3.md#what-it-models) says.
- `glitch`, with `peripheral` `I2C0`: with the filter on, SCL or SDA held a level for fewer sclk cycles than its threshold while a transfer ran.
- `reset_while_sending`, with `peripheral` `I2C0`: a gate store cleared I2C0's clock bit or set its reset bit while a transfer ran.

The access writes nothing, apart from an `app_cpu_without_reset` or `pro_cpu_stall` store, whose word keeps the value it stored. An `app_cpu_without_reset` at the 1,000-cycle bound is not a store. The run keeps every row before it, as [esp32s3.md](esp32s3.md#what-it-models) says.

## chip.halted

Severity `error`. Refs are the chip. `t_ps` is the time the core reached the jump. With both cores started, it is the time the last core that could run stopped. It ends the run.

The message is `{part} halted at {pc}, a jump to itself that no interrupt or watchdog can leave, at {t_ps} ps`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `pc` and `symbol`, as `chip.unsupported_instruction` writes them. The core reached a `j` to its own address while every interrupt `INTENABLE` enabled was at or below the current interrupt level, `PS.INTLEVEL` or `EXCMLEVEL` while `PS.EXCM` is set, the interrupt matrix mapped no source to that core's CPU interrupt 14, the non-maskable one, and no watchdog ran. ESP-IDF's panic handler ends in such a loop when the app is built to print and halt, so a panic there ends the run with this error. A bare-metal program that ends in such a loop does too.

## chip.unmapped_fetch

Severity `error`. Refs are the chip. `t_ps` is the time of the fetch. It ends the run.

The message is `{part} fetched code at {pc}, where the model maps no memory, at {t_ps} ps`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `pc` and `symbol`, as `chip.unsupported_instruction` writes them. An `esp32s3` fetches only from its instruction-bus SRAM window, as [esp32s3.md](esp32s3.md#what-it-models) says.

## chip.watchdog

Severity `error`. Refs are the chip. `t_ps` is the time the watchdog's stage ran out. It ends the run.

The message is `{part} {watchdog} watchdog's stage {stage} ran out and reset the {reset} at {t_ps} ps; the firmware neither fed nor disabled it`.

`data` holds `watchdog`, which is `RTC`, `timer group 0` or `timer group 1`, `stage`, 0 to 3, and `reset`, which is `CPU`, `system` or `RTC domain`. [esp32s3.md](esp32s3.md#what-it-models) says when each runs and what each stage does.

## chip.efuse_program

Severity `error`. Refs are the chip. `t_ps` is the time of the store. It ends the run.

The message is `{part} started an eFuse program command at {pc}, which would burn eFuses, at {t_ps} ps`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `pc` and `symbol`, as `chip.unsupported_instruction` writes them. The store wrote `EFUSE_PGM_CMD` to `EFUSE_CMD_REG`. The model never burns its eFuses, and the store writes nothing.

## chip.flash_command

Severity `error`. Refs are the chip. `t_ps` is the time of the store. It ends the run.

The message is `{part} {what} at {pc}, {why}, at {t_ps} ps`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it. `{what}` and `{why}` follow `reason`, and a command prints as `command 0x20`.

`data` holds `command`, `pc`, `reason` and `symbol`. `command` is the command byte SPI1 sent, or `null` when no one byte went. `pc` and `symbol` are as `chip.unsupported_instruction` writes them. The store writes no status bit and no flash byte. `reason` is one of these.

- `unsupported`: a program, an erase, or another command the model does not run. The message reads `sent the flash command 0x20`, `which the model does not run`.
- `status_write`: the fixed `WRSR`, or a user `0x01`, `0x31` or `0x11`, with or without `WREN`. A status write can set the flash's protect and one-time lock bits. The message reads `sent the flash the status write command 0x01`, `which could lock the flash`.
- `no_command_phase`: a user command with no command phase, or a write that sets only `SPI_MEM_FLASH_PE`. `command` is `null`.
- `start_bits`: one write of `SPI_MEM_CMD_REG` that starts more than one command, such as `WREN` with a sector erase. None of them runs. `command` is `null`.
- `past_end`: a read whose bytes reach past the flash's end. A chip with no flash refuses every read. The message reads `sent the flash the read command 0x03`, `which reaches past the flash's end`.
- `address_bits`: a command whose address phase is longer than 32 bits.
- `spi0`: a write of SPI0's `SPI_MEM_CMD_REG` that starts a command. `command` is `null`.
- `pads`: a write that would take one of the flash's pads, GPIO26 to GPIO32, from SPI0 and SPI1. It gives the pad an IO_MUX function other than 0, sets its `GPIO_ENABLE` bit, or moves its `GPIO_FUNCn_OUT_SEL_CFG_REG` from its reset value, as [esp32s3.md](esp32s3.md#what-it-models) says. `command` is `null`. The message reads `took a flash pad from SPI0 and SPI1`, `which could send the flash any command`.
- `cs`: a user command while `SPI_MEM_MISC_REG` has `CS0_DIS` set or `CS1_DIS` clear. The register resets to `0x2`, so CS0 is on and CS1 is off. The fixed commands do not look at it. The message reads `sent the flash command 0x03 with CS0 disabled or CS1 enabled`, `which the model does not run`.

The flash reads, and reports its ID and status, as [esp32s3.md](esp32s3.md#what-it-models) says.

## chip.unsupported_clock

Severity `error`. Refs are the chip. `t_ps` is the time of the store. It ends the run.

The message is `{part} picked a CPU clock the model does not run, the RC fast clock or a reserved setting, at {pc} at {t_ps} ps`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `pc` and `symbol`, as `chip.unsupported_instruction` writes them. The store to `SYSTEM_SYSCLK_CONF_REG` or `SYSTEM_CPU_PER_CONF_REG` left `SYSTEM_SOC_CLK_SEL` at the RC fast clock or its reserved value, or the PLL with the reserved `SYSTEM_CPUPERIOD_SEL` 3, or with `SYSTEM_CPUPERIOD_SEL` 2 while `SYSTEM_PLL_FREQ_SEL` picks the 320 MHz PLL.

## chip.restart

Severity `warning`. Refs are the chip. `t_ps` is the time of the store.

The message is `{part} reset itself at {pc} at {t_ps} ps; the model does not run the boot ROM, so the chip stays in reset`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `pc` and `symbol`, as `chip.unsupported_instruction` writes them. The store set `RTC_CNTL_SW_PROCPU_RST` or `RTC_CNTL_SW_SYS_RST` in `RTC_CNTL_OPTIONS0_REG` while `RTC_CNTL_STORE6_REG` held 0 or the hint `ESP_RST_SW`, as `esp_restart` leaves it. From that cycle the core runs no further and each pad drives as at power-on, while the rest of the circuit runs on to `stop_ps`. Any other reset is `chip.fault_restart`.

## chip.fault_restart

Severity `error`. Refs are the chip. `t_ps` is the time of the store. It ends the run.

The message is `{part} reset itself at {pc} {why}, at {t_ps} ps`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it. `{why}` follows `reason`, such as `after a panic`.

`data` holds `pc`, `reason` and `symbol`. `pc` and `symbol` are as `chip.unsupported_instruction` writes them. The store set `RTC_CNTL_SW_PROCPU_RST` or `RTC_CNTL_SW_SYS_RST`, and `RTC_CNTL_STORE6_REG` held what ESP-IDF records before such a reset, `hint | hint << 16 | 1 << 31`, as [esp32s3.md](esp32s3.md#what-it-models) says. `reason` is one of these.

- `panic`: the hint `ESP_RST_PANIC`, which IDF's panic handler records after an abort, an assert or an unhandled exception.
- `interrupt_watchdog`: `ESP_RST_INT_WDT`.
- `task_watchdog`: `ESP_RST_TASK_WDT`.
- `watchdog`: `ESP_RST_WDT`.
- `brownout`: `ESP_RST_BROWNOUT`.
- `unreadable`: any other word but 0, or no `RTC_CNTL_STORE6_REG` at all, as on a bare-metal chip. The message reads `with no reset reason the model can read`.

Two resets that are no fault on the board end the run here as `unreadable`: a bare-metal program's software reset, and `esp_restart` after a deep-sleep wake stub left its address, whose top bit is clear, in `RTC_CNTL_STORE6_REG`, which IDF reports as `ESP_RST_SW`.

## chip.unmapped_load

Severity `error`. Refs are the chip. `t_ps` is the time of the load. It ends the run.

The message is `{part} loaded from {address} at {pc}, where the MMU maps no flash, at {t_ps} ps`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `address`, `pc` and `symbol`. `address` is the address the load read, in the instruction or data cache window. `pc` and `symbol` are as `chip.unsupported_instruction` writes them. The MMU entry of that page is invalid, or picks PSRAM, which the chip lacks, or maps a page past the flash's end, or the chip has no flash, as [esp32s3.md](esp32s3.md#what-it-models) says.

## chip.unmodeled_register

Severity `warning`. Refs are the chip. `t_ps` is the time of the first access to the register, or for a register the model keeps without running its effect, the time of that write.

The message is `{part} read {register} at {address}, which the model does not keep, first at {t_ps} ps from {pc}`, with `wrote` for a write and ` in {symbol}` after the PC when the firmware has a symbol at or below it. For a register the model keeps without running its effect, it reads `{part} wrote {register} at {address}, whose effect the model does not run, at {t_ps} ps from {pc}`.

`data` holds `address`, `kept`, `pc`, `register`, `symbol` and `write`. `address` is the register's word address. `register` names it by its block and the offset into the block, such as `UART0+0x0` or `TIMG0+0x24`. `pc` is the address of the instruction that made the access, and `symbol` is written as `chip.unsupported_instruction` writes it. `write` is true for a store. A read of such a register gives 0 and a store is dropped. The run reports each register once per chip, at its first access, whatever later accesses do. The registers whose writes change the slow clock or SPI0's read command, and the clock gate and reset bits of the peripherals the model keeps without running, are kept, and their effect does not run. `kept` is true for those and false for every other register. The run reports each write that changes one of those, every time, as [esp32s3.md](esp32s3.md#what-it-models) lists. For the peripherals' gate and reset words that is a write that changes a bit of SYSTIMER, TIMG0, TIMG1, SPI0 and SPI1, the eFuse controller, the USB Serial/JTAG unit, `SYSTEM_TIMERS` or `SYSTEM_WDG`. Such a write is listed once, when it lands, so a write that the core never runs again after it stops there is not listed. The list is the work queue of registers a firmware needs, as [esp32s3.md](esp32s3.md#unmodeled-registers) says.

## trace.too_large

Severity `error`. No refs. For an `orbe-run` build, `t_ps` is `null`.

The message is `the grid gives {rows} rows; the trace holds {limit} rows of {columns} columns; raise grid_ps or shorten stop_ps`.

`data` holds `columns`, `limit` and `rows`. `rows` is `ceil(stop_ps / grid_ps) + 1`, the rows the grid gives. `limit` is the row cap, `min(1000000, 67108864 / columns)`. 67108864 is 2^26, so the stored f64 values of `analog.f64` stay within 512 MiB. `envelope.f64` adds at most twice that again, and writing the folder keeps a second copy of both. `columns` is the number of trace columns.

The run checks the count before it solves anything, so a refused run writes `{}` to `observations.json` and no `traces/`. The rows depend only on `stop_ps` and `grid_ps`. Edges, jumps, chip events and the error control add steps, not rows, so an `orbe-run` build that passes the check never runs out of rows. The test-only step-rows mode can also stop mid-run with `t_ps` the step that did not fit, keeping its rows.

## trace.row_interpolated

Severity `warning`. No refs. `t_ps` is the row's time.

The message is `the row at {t_ps} ps did not solve; it is the line between the steps at {from_ps} ps and {to_ps} ps`.

`data` holds `from_ps` and `to_ps`, the accepted steps on either side of the row. A row no step lands on is a probe from the last accepted step, as [engine.md](engine.md#rows) says. When its solve fails, it is solved once more with Newton started from the step after it. When that fails too, every column of the row is the straight line from the state the probe was solved from at `from_ps` to the step at `to_ps`, read at `t_ps`. After a restart at `from_ps` that state is the restart's solve, not the step's row. Every column of it takes the new source levels and contacts, a current source's and a switch's included. The run goes on. No step, stop, edge or other diagnostic changes, and a grid with no row between those two steps has no such warning.

## rating.current

Severity `warning`. Refs is the rated quantity, `#<part>.i` (a voltage source's too), `#<part>.ic` for a bipolar transistor, `#<part>.id` for an `nmos`, `#<part>.iout` for a linear regulator, or a chip pin, `#<part>.pin.<pin>`, for an `atmega328p`. `t_ps` is the first time over the rating, 0 for the operating point.

The message is:

```text
{ref} peaks at {peak} A, over its {limit} A rating
```

`data` holds `limit`, `over_ps`, `peak`, and `peak_ps`. `limit` is the rating. `peak` is the largest magnitude of the quantity. `peak_ps` is when it peaked. `over_ps` is `null` for an operating point, which has no duration.

A transient checks every accepted step, not only the rows. Each step's value holds until the next accepted step. `t_ps` is the first step over the rating, and `over_ps` adds the time to the next step for every step over it. The last step adds nothing. So `t_ps` and `peak_ps` can fall between rows. At a switch edge the solve at the edge holds the old contact. With 22 ohm in place of 220 ohm in `packages/engine/tests/circuits/button-led`, `#led1.i` is first over 1 ns after the press, and `over_ps` is exactly the time the contact was closed.

A quantity is over when its magnitude is above the rating. Equal is not over. `peak` is the largest magnitude. For an operating point it equals `abs(observations.op[ref])`. The operating point is still written, and `orbe-run run` exits 0.

The 9 V LED example puts about 0.55 A through an LED rated 0.03 A.

A voltage source with `ratings.current` names its current, `#<part>.i`, over that rating. The USB sources of the catalog's boards carry 0.5 A, the most a high-power USB 2.0 function draws (Table 7-7, `ICCHPF`). A 2.5 ohm load from a Nano's 5V to its GND draws 2 A, and `#nano__usb__host.i` is over.

An `atmega328p` with `ratings.pin_current` names each port pin over it, such as `#u1.pin.PB1`. A pin's current is the current out of the pin into the circuit, through its output stage in the state the firmware drives, its pull-up and its leakage to both rails, as [netlist.md](netlist.md#atmega328p) stamps them. With `ratings.supply_current`, `#u1.pin.VCC` is the sum of every port pin's current drawn from `VCC`, and `#u1.pin.GND_8` the sum of every port pin's current into `GND_8`, which carries `GND_22` too. The core draws no supply current of its own, so neither sum holds one. A row is solved with the drive the firmware had in force until that step, so a pin that changes drive on a step is checked with its new drive from the next step on.

The number is the model's. The stage is linear, 25 ohm from its rail, so a pin shorted to the other rail reads 200 mA at 5 V. A real driver saturates, and a short draws about half that. Both are far over the 40 mA rating, so the warning holds. The rating is the datasheet's absolute maximum, which [avr.md](avr.md#ratings) names. A pin over it can be damaged.

A Nano running a sketch that drives D9 high into 47 ohm to ground reads:

```text
#nano__mcu.pin.PB1 peaks at 0.06944434799653014 A, over its 0.04 A rating
```

Six pins each driving 120 ohm to ground carry 34.5 mA each, under the pin rating, and 207 mA together, so `#nano__mcu.pin.VCC` is over its 0.2 A.

## rating.power

Severity `warning`. Refs is the rated quantity, `#<part>.p`. `t_ps` is the first time over the rating, 0 for the operating point.

The message is:

```text
{ref} peaks at {peak} W, over its {limit} W rating
```

`data` is the same as `rating.current`.

The 9 V LED example reports `#led1.p` over its 0.075 W rating and `#r1.p` over its 0.25 W rating.

## rating.voltage

Severity `warning`. Refs is the chip pin, `#<part>.pin.<pin>`. `t_ps` is the first time the pin is outside its window, 0 for the operating point.

The message is:

```text
{ref} reaches {peak} V over the chip's ground, outside its {low} V to {high} V rating
```

A window with no floor ends `outside its {high} V ceiling`, and one with no ceiling `outside its {low} V floor`.

`data` holds `high`, `low`, `over_ps`, `peak`, and `peak_ps`. Each voltage is over the chip's ground pin, `GND_8` on an `atmega328p`. `peak` is the pin's voltage where it was farthest outside the window, and `peak_ps` is when. `low` and `high` are the window at that time, and either is `null` when the part rates none. `over_ps` is `null` for an operating point, which has no duration, and counts the time outside the window as `rating.current` counts the time over.

An `atmega328p` with a `ratings` object checks every pin but its two ground pins. The floor is `ratings.rail_margin` below ground. The ceiling is `ratings.rail_margin` above the chip's own `VCC`, so it follows the supply. `VCC`'s ceiling is `ratings.supply_voltage`, and `PC6`'s is `ratings.reset_voltage` while it is `RESET`. A pin exactly on the edge is inside. The model has no clamp diodes, so a pin outside the window draws no clamp current and its voltage is what the circuit drives. A real pin conducts into its clamp there, which is the damage the rating guards against.

A Nano with a 9 V source through 1 kohm on A0 reads:

```text
#nano__mcu.pin.PC0 reaches 8.997637213833697 V over the chip's ground, outside its -0.5 V to 5.5 V rating
```
