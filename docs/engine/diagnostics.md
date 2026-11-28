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

A session's `diagnostics()` returns the warnings found so far, and once the session ends they are that list's. While it runs, a `Pulses` record still in flight is not yet counted, as [engine.md](engine.md#sessions) says. The folder's warnings are `net.floating`, `trace.row_interpolated`, `rating.current`, `rating.power`, `rating.voltage`, `rating.supply_low`, `rating.temperature`, `regulator.current_limit`, `mosfet.body_diode`, `chip.protocol`, `chip.link_miswired`, `chip.floating_input`, `chip.unmodeled_register` and `chip.restart`. `diagnostics()` returns all but `trace.row_interpolated`. It leaves out `trace.row_interpolated`, because that warning names a grid row whose probe failed twice, and only a recorded trace defines those rows. An open session has no grid. The accepted steps do not depend on the grid, and neither do the warnings `diagnostics()` returns. Every other code on this page is an error. The error that ends a run is `fault` on the session, and `diagnostics()` does not return it. [engine.md](engine.md#sessions) says when the samples are taken.

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

`data` holds `found` and `supported`. `supported` lists every version this engine reads, `[1]`. The message joins that list with `, `.

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

`data` holds `field`, `rule`, and `value`. `rule` is `positive`, `finite_reciprocal`, `finite_junction`, `bounce_room`, `non_negative`, `fraction`, `below_one`, `above_high`, `hysteresis`, `above_resistance`, `above_drop`, `unsupported`, `span`, `leds`, `delay`, `range`, `finite`, or `distinct`. The message prints `value` in the same form as `data.value`, but for `unsupported`, which prints the byte, `leds`, which prints the count as an integer, and a value that is not finite, which `data.value` holds as `null` and the message prints as `NaN`, `inf` or `-inf`. Resistance must be positive, and its reciprocal must be finite. A value that breaks both reports `positive`. A potentiometer's `resistance` follows those rules. Its `position` must be inside (0, 1), which breaks `fraction`. A leg, `position * resistance` or `(1 - position) * resistance`, that breaks the resistance rules is `netlist.value` on field `resistance`, and `value` is that leg. A source's voltage or current, and each level of its waveform, only has to be finite and in its unit. The diode and LED fields follow the resistance rules. `saturation_current` and `emission_coefficient` also follow `finite_junction`, as [netlist.md](netlist.md) defines it. A zener's seven fields follow the resistance rules. `zener_impedance` must exceed `series_resistance + breakdown_resistance`, which breaks `above_resistance`. `zener_voltage` must exceed `zener_current` times that sum, which breaks `above_drop`. The derived breakdown junction follows `finite_junction`, naming `zener_impedance` or `zener_voltage` as [netlist.md](netlist.md#zener) says. An inequality that fails is reported alone, without `finite_junction`. A linear regulator's `output_voltage`, `output_resistance`, `dropout_voltage`, `dropout_resistance` and `current_limit` follow the resistance rules. Its `line_regulation` must be at least 0 and below 1, which breaks `below_one`. Its `quiescent_current` and `ground_current_ratio` must be at least 0, which breaks `non_negative`. Every `npn` and `pnp` field follows the resistance rules. `saturation_current` with `forward_emission`, and `saturation_current` with `reverse_emission`, follow `finite_junction`, which names the emission of the junction that breaks it, as [netlist.md](netlist.md#npn-and-pnp-transistors) says. An `nmos` or `pmos` `transconductance_parameter` and its three capacitances follow the resistance rules. Its `threshold_voltage` only has to be finite and in volts. Its `channel_length_modulation` must be at least 0, which breaks `non_negative`, and a wrong unit on that field reports `netlist.unit` alone. [netlist.md](netlist.md#n-channel-mosfet) says why. A rating must be positive, with field `ratings.current` or `ratings.power`, or on an `atmega328p` `ratings.pin_current`, `ratings.supply_current`, `ratings.rail_margin`, `ratings.reset_voltage` or `ratings.supply_voltage`. Capacitance and a switch's two resistances follow the resistance rules. Every relay field follows the resistance rules, and a `drop_out_current` at or above `pull_in_current` breaks `hysteresis` on `drop_out_current`. `bounce.window` and `bounce.max_bounces` must be positive, and `bounce.window` must be at least `2 * max_bounces * 2000`, which breaks `bounce_room`. An NE555's resistances follow the resistance rules, with dotted fields such as `discharge.on_resistance`. Its `reset_threshold` must be positive, and `output.high_drop` and `output.low_drop` must not be negative. An ATmega328P's output stage follows the NE555's, and its two pull-ups the resistance rules. `clock_hz` must be positive. Each of `input_threshold.low`, `input_threshold.high`, `reset_threshold.low` and `reset_threshold.high` must be inside (0, 1), which breaks `fraction`, and a `low` above its `high` breaks `above_high` on `low`. A fuse byte that selects a clock other than a crystal or ceramic resonator, or programs `CKOUT`, is `unsupported` on `fuses.low`. One that programs `DWEN` or `BOOTRST` is `unsupported` on `fuses.high`, and one with a reserved `BODLEVEL` is `unsupported` on `fuses.extended`. [netlist.md](netlist.md#atmega328p) says why. `value` is then the byte. Its `leakage_resistance` follows the resistance rules. An `ssd1306` `address` other than 0x3C or 0x3D is `unsupported` on `address`, and `value` is that address. An `hd44780_i2c` `address` outside 0x20 to 0x27 is `unsupported` on `address`, and `value` is that address. A `cst816` coordinate above 4095 breaks `span` on `touches[n].x` or `touches[n].y`. A `ws2812` `leds` from 0 to 65535 outside 1 to 1024 breaks `leds`, and any other number is `netlist.syntax`. Its `dout.delay` above 1000000 ps breaks `delay`, and the message prints it as an integer. A world quantity's value breaks `range` on ref `#world.<quantity>`, with field `value`, `initial` or `events[k].level`. A world-driven field keeps its own rules at both ends of its quantity's range, on field `<field>.curve at <end> <unit>`, and breaks `finite` there when the curve is not finite, for example `#r1 resistance.curve at 125.0 degC is -1250.0; it must be positive`. A `power` curve's `x` follows the resistance rules' `positive`. A `beta` curve's `x` outside the temperature range breaks `range`. A `linear` curve whose `x1` equals its `x0` breaks `distinct` on `<field>.curve.x1`. A number outside the f64 range is `netlist.syntax`, and so is a non-integer `bounce` value, a non-integer or oversized `clock_hz`, and a fuse byte that is not an integer from 0 to 255.

## netlist.firmware

Severity `error`. Refs is the part. `t_ps` is `null`.

The message is `{ref} firmware line {line} {what}`, or `{ref} firmware {what}` when `line` is `null`. `{what}` says the rule.

`data` holds `field`, `line`, and `rule`. `field` is `firmware`. `line` is the 1-based line of the Intel HEX text at fault, or `null` when no one line is. `rule` is one of these.

- `record`: the line is not an Intel HEX record. It has no `:`, a digit that is not hexadecimal, an odd number of digits, or a length that does not match its byte count, or an address record is not the length its type takes, or an end-of-file record has data.
- `checksum`: the record's bytes do not sum to 0.
- `record_type`: the type is not 00 to 05.
- `after_end`: a record follows the end-of-file record.
- `no_end`: the text has no end-of-file record. `line` is `null`.
- `empty`: the text has no data record, so the flash would be all erased. `line` is `null`. The `.eep` file a build writes beside the `.hex` holds only the end-of-file record when the sketch has no EEPROM data.
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

- `unknown`: the name is not a row of the [world table](netlist.md#world). The message is ``{field} names `{name}`, which is not a world quantity; the quantities are temperature, humidity, light, pressure``, with the ref before it for a part's field, for example ``#r1 resistance.quantity names `lite`, which is not a world quantity; the quantities are temperature, humidity, light, pressure``. `field` is `world` for a name in `world` and `<field>.quantity` for a part's.
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

A DC path is a chain of resistors, potentiometers, voltage sources, diodes, LEDs, zeners, linear regulators, transistors, inductors, relays, and switches from the net to ground. A resistor, voltage source, diode, LED, zener, inductor, or switch joins its two pins. A linear regulator joins `in` to `gnd` and `out` to `gnd`. An inductor joins them through its series resistance, because its inductance is a short at DC. A relay joins `coil_a` to `coil_b` through its coil, and `com` to `no` and `com` to `nc` in either state, so a net on a contact's open side is not floating. A potentiometer joins `a` to `wiper` and `wiper` to `b`. An `npn` or a `pnp` joins `collector` to `base` and `base` to `emitter`, through its junctions. An `nmos` or a `pmos` joins `drain` to `source`. Its gate joins nothing, so a net reached only through gates and capacitors is `net.floating`. A switch joins its pins open or closed. An NE555 joins VCC to CONT, CONT to GND, VCC to GND, OUT to VCC, OUT to GND, and DISCH to GND, in either latch state, and joins nothing to TRIG, THRES, or RESET. An ATmega328P joins each port pin, `PC6` included, to VCC and to `GND_8`, through the leakage it stamps in every drive state. Its `GND_22` is on the net of its `GND_8`. It joins nothing to `PB6`, `PB7`, `AREF`, or `AVCC`, as [netlist.md](netlist.md#atmega328p) explains. An SPI replay joins `CS`, `SCLK` and `D0` to `D3` each to `VCC` and to `GND`, through the stage it drives. A capacitor with `"initial": null` does not join its pins, because it is open at DC. Neither does a current source. A capacitor with an `initial` voltage joins them, because the operating point holds it. gmin, 1e-12 S from every node to ground, is separate from that chain.

`net.floating` is a static check on the netlist's topology. It never depends on a solve, so its `t_ps` is always `null`. Because a port pin's leakage joins its net to both rails, a net that only a chip's input and an open button hold is never `net.floating`. A run whose firmware reads such an input reports [`chip.floating_input`](#chipfloating_input).

A resistor from `vin` to `lonely`, with a voltage source from `vin` to `gnd`, puts `lonely` on a path to ground. Two LEDs in series from `vled` to `gnd`, with a resistor from `vin` to `vled`, put `mid` on a path to ground. A net that no part names is floating. Two nets whose only part is a resistor between them are floating. The operating point is still written, and `orbe-run run` exits 0.

## net.short

Severity `error`. Refs are the voltage source, then `#net.<name>`, the net both its pins are on. `t_ps` is `null`.

The message is:

```text
{part} holds `p` and `n` apart, but both are on {net}, so the circuit shorts it
```

`data` is `{}`.

Parse checks it, so the netlist is refused and nothing solves. An ideal source across one net has a branch row of zeros, which the solve would report as `solver.singular` at the source. A wire from a board's 5V straight to its GND does this on a board whose USB source feeds 5V with nothing in between. A source of 0 V across one net is the same short.

## solver.singular

Severity `error`. Refs is the unknown the elimination found no pivot for, a net or a part: every entry left in its column is zero, or within the rounding of the sums that made it. `t_ps` is 0 for the operating point. In a transient it is the end of the step that failed, the probe's time for a probe locating a chip event, or the restart's time for the restart solve after a switch edge, a source jump or a chip drive change.

The message is `singular matrix at {ref}`.

`data` is `{}`.

A voltage source whose two pins are on one net never gets here. Parse refuses it as [net.short](#netshort). Two voltage sources in parallel, set to different voltages, are singular. With `v1` at 5 V and `v2` at 3 V, both from `vin` to `gnd`, the ref is `#v2`, with or without a diode beside them.

## solver.nonfinite

Severity `error`. Refs is the unknown or the part. `t_ps` is 0 for the operating point. In a transient it is the end of the step that failed, or the restart's time for the restart solve after a switch edge, a source jump or a chip drive change.

The message is `non-finite number at {ref}`.

`data` is `{}`.

A stamped value, a factor or a solution component that is not finite names that unknown, in a linear circuit or a nonlinear one alike. No change of gmin or source scale makes it finite, so no fallback retries it. A 1e200 V source across 1e-300 ohm overflows the solution at `#net.vin`, with or without a diode after the resistor. An observed `ib`, `ic`, `id`, `iin`, `iout`, `i`, `v`, or `p` that is not finite names the part. A 1e200 V source across 1 ohm overflows `p` on the resistor.

## solver.too_large

Severity `error`. No refs. `t_ps` is 0 for the operating point.

The message is `{unknowns} unknowns exceeds the solver limit of {limit}`.

`data` holds `limit` and `unknowns`. `limit` is 4096. The solver checks the count before it allocates any system, at t = 0, so `observations.json` is `{}`.

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

The operating point starts with Newton at nominal conditions. If that fails, gmin stepping adds a conductance to ground from every node but a bipolar transistor's internal ones and the internal nodes of a diode, an LED or a zener, from 1e-2 S down by decades, and ends at nominal. An `nmos` or a `pmos` has no internal node, so the stepping reaches its drain, gate and source. If that fails, source stepping scales every source from 0 up to 1. This code means all three failed. `iterations` counts Newton iterations over all of them. `gmin` is the smallest added conductance that converged, in S, or `null`. `source_scale` is the largest scale that converged, or 0.

The restart solve runs the same ladder, with every capacitor held at its voltage, every inductor held at its current, and the new contacts, source levels and chip states in force. A restart solve follows each switch edge and source jump, and each chip event whose new state stamps differently, at its time. A flip that changes no drive takes none. Its `data` is the same as the operating point's. Its message names the restart and its time:

```text
Newton did not converge at {ref} after {iterations} iterations in the restart solve at {t_ps} ps; source stepping reached {source_scale}
```

A fallback that succeeds is not reported. Its answer converged at nominal conditions, the same as plain Newton's. A singular matrix at any step ends the run as `solver.singular`, and a value that is not finite as `solver.nonfinite`.

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

## solver.step_work

Severity `error`. No refs. `t_ps` is the last accepted time, which is the trace's `end_ps`.

The message is:

```text
one step took more than {limit} solver operations, its limit; the circuit is too large or too stiff to step
```

`data` holds `limit`. The limit is 1073741824 multiply-adds, 2^30, about a second of solving on one core. It counts the linear solves toward one accepted step: every try, retry and probe since the last accepted step. The run keeps the observations and the last row up to that time.

## chip.unsettled

Severity `error`. Refs are the chips whose state changed on the last solve, in part order. `t_ps` is 0 for the operating point, or the time of the restart solve.

The message is `the chips kept changing state for {rounds} solves at {t_ps} ps`.

`data` holds `rounds`, which is 8. At one time the run solves, lets every chip react to the levels on that solve, and solves again while any state changed. A chip whose output drives its own inputs with nothing to slow it never agrees with its levels. An NE555 with OUT wired to TRIG and THRES is one: low sets the latch and high resets it. So are two NE555s, each OUT on the other's TRIG, that reset at one time. So is a relay whose `nc` feeds its own coil, when that coil is powered at t = 0. Open, the coil current is above pull-in. Closed, it is below drop-out. When the coil is powered later, it holds its current across each move and the contact buzzes. So is a supply-gated chip whose outputs or device draw, once powered, pull its own supply below its release supply while releasing lets it rise past its minimum, at t = 0 or at a later edge with no capacitance on that supply. At t = 0 `observations.json` is `{}`.

## chip.protocol

Severity `warning`. Refs are the chip. `t_ps` is the first time that rule fired. A dropped SPI transaction uses the end of that transaction.

The message for a dropped transfer is `{part} dropped a transfer that broke rule {rule} at {t_ps} ps, the first time`.

`data` holds `rule`. A chip that takes `Spi` records drops one that breaks its protocol, and the run reports each rule once per part, at its first time. An `st77916` has two rules, which [netlist.md](netlist.md#st77916) describes. An `ssd1306` breaks `command` with a command the datasheet does not list, addressing mode 3, a multiplex ratio below 15, or a control byte with any of its low six bits set, and also has rules `scroll` and `read`, which [netlist.md](netlist.md#ssd1306) describes. A `cst816` has one more. Rule `sda_while_scl_high` means an `SDA` change came due while `SCL` was high, so the target left `SDA` where it was and went idle. A release it refused waits for the next `SCL` fall. A release while the part is in reset is not this rule. Its message is `{part} left SDA unmoved because SCL was high at {t_ps} ps, breaking rule sda_while_scl_high, the first time`. [engine.md](engine.md#i2c-targets) describes the hold. A `ws2812` has one more. Rule `bit` means a high, or a low after a high that was in a window, fell outside the chain's bit times, so that bit was dropped. Its message is `{part} dropped a bit that broke rule bit at {t_ps} ps, the first time`. [engine.md](engine.md#records) has the windows. A `dht22` has one more. Rule `interval` means a start came inside the sensor's 2 s warm-up, or within 2 s of the last start it answered, so the sensor did not answer it. Its message is `{part} did not answer a read that broke rule interval at {t_ps} ps, the first time`. [netlist.md](netlist.md#dht22) has the times.

| `rule` | What broke |
| --- | --- |
| `command` | Its command word is cut short by a release of `CS` or followed by phases, its opcode is not the one that command takes, its data lanes after the word do not match that opcode, a transaction with phases comes while a command is open, or its register command is short of bytes or out of range. A continuation whose data lanes do not match drops that transaction's data, and a release still closes the open command. Opcode 0x02 writes a register on one lane. Opcode 0x32 writes pixels on four lanes, and only `RAMWR` and `RAMWRC` take it. |
| `pixel_format` | It starts a pixel write while the pixel format is not 16 bits a pixel. |
| `sda_while_scl_high` | An I2C target's `SDA` change came due while `SCL` was high. The target goes idle. A refused release waits for the next `SCL` fall. A release in reset is not this rule. |
| `bit` | A `ws2812` high was in neither the T0H window, 220 ns to 400 ns, nor the T1H window, 580 ns to 1 µs. That bit is dropped, and its time is the fall that ended the high. The chain reports the first one. |
| `interval` | A `dht22` start came within 2 s of power-on or of the last start it answered. The sensor does not answer it, and its time is the end of the start's low. |
| `spi` | A `bmp280` saw `CSB` low, so it is in SPI mode and does not answer on I2C. The time is the first sample that read `CSB` low. |
| `spi3w` | A `bmp280` took a `config` write with `spi3w_en` set. 3-wire SPI is not modelled, and the time is that write. |
| `config_in_normal` | A `bmp280` ignored a write to `config` (0xF5) that would change it while `ctrl_meas` was in normal mode (BST-BMP280-DS001 §4.3.5). Write sleep mode to `ctrl_meas` (0xF4) before `config`. A write that leaves `config` unchanged, or a write in sleep or forced mode, stays silent. The time is the first such write. |
| `supply` | An `sg90` supply was at least 3.0 V and below 4.5 V. The shaft keeps moving. Below 4.8 V the speed is the 4.8 V figure times the supply over 4.8 V. 4.5 V is the low end of the range the Nano's 5 V pin can read, so a pin in that range does not warn. Below 3.0 V the shaft does not follow and the draw is none, and a supply that returns above 3.0 V follows the next pulse. The part reports the first sample in the slow range. A supply pulled under 3.0 V by the moving draw is `supply_sag` instead. |
| `supply_sag` | An `sg90` tried to move and the supply under that moving draw was under 3.0 V. The shaft holds. The message says the supply sags under the servo's moving draw. The draw is the 5 mA hold once the rail has recovered, so a read does not sit on the collapsed current for a solver step. A supply that stays under 3.0 V with the moving draw already dropped draws none and does not raise this rule. A real SG90 browns out and jitters there, because its controller resets. The model holds the shaft instead of buzzing. |
| `scroll` | An `ssd1306` took a scroll command, `26h`, `27h`, `29h`, `2Ah`, `2Fh` or `A3h`, which it does not run. Its message is `{part} did not run a scroll command, breaking rule scroll at {t_ps} ps, the first time`. |
| `read` | A master read from an `ssd1306`, which has no status byte here. It answers 0xFF bytes. Its message is `{part} answered a read with 0xFF bytes, breaking rule read at {t_ps} ps, the first time`. |
| `bus_width` | An `hd44780_i2c` received an 8-bit command or a function set selecting the 8-bit bus (`DL = 1`), which the PCF8574 backpack cannot drive. |
| `font` | An `hd44780_i2c` or `hd44780` received a function set selecting one line of the 5x10 font (`N = 0`, `F = 1`), which is not supported on a 16x2 glass. With two lines `F` is don't care. |
| `enable_cycle` | An `hd44780` `E` rose sooner after its last rise than `tcycE`, 500 ns from 4.5 V and 1000 ns below (ADE-207-272 pp. 52, 49). The cycle still runs. |
| `enable_width` | An `hd44780` `E` high was shorter than `PWEH`, 230 ns from 4.5 V and 450 ns below. The cycle still runs. |
| `setup` | An `hd44780` `RS` or `RW` moved less than `tAS` before `E` rose, 40 ns or 60 ns, or a write's data less than `tDSW` before `E` fell, 80 ns or 195 ns. The cycle still runs. |
| `hold` | An `hd44780` `RS` or `RW` moved less than `tAH` after `E` fell or while `E` was high, 10 ns or 20 ns, or a write's data less than `tH` after `E` fell, 10 ns. The cycle still runs. |
| `contrast` | An `hd44780` had its display on with `VDD - V0` under its `lcd_voltage`, so its glass shows no characters. Its time is the first such moment. |
| `init_busy` | An `hd44780_i2c` or `hd44780` received a command or data write inside its HD44780U internal reset's busy time, 10 ms after `VCC` reaches 4.5 V (ADE-207-272 p. 23), or the whole 40 ms wait after `VCC` reaches 2.7 V below it (Figs. 23–24). The controller drops the write, and the message says so. Its time is the first such `E` falling edge. |
| `init_wait` | An `hd44780_i2c` or `hd44780` received a command or data write after its internal reset's busy time and before its power-on reset wait elapsed, 15 ms after `VCC` reaches 4.5 V (Figs. 23–24). The controller runs the write, and the message says it ran early. Its time is the first such `E` falling edge. |
| `low_supply` | An `hd44780` started a bus cycle, an `E` rise, with `V(VDD) - V(VSS)` under its `operating_voltage`, the module's minimum operating supply, and at or above the controller's 2.7 V. The cycle still runs as the controller runs at that supply. Its time is the first such rise. |
| `input_level` | A `max7219` driven logic input (`DIN`, `CLK`, or `LOAD`) stayed between its `VIL` (0.8 V) and `VIH` (3.5 V) thresholds longer than a normal edge transition (500 ns) while powered, where the datasheet does not guarantee the logic level (for example, 3.3 V logic driving a 5 V MAX7219 without a level shifter). Undriven high-Z pins during MCU boot are not flagged. |

## chip.link_miswired

Severity `warning`. Refs are the chip that dropped a transaction. `t_ps` is the end of the first transaction it dropped.

The message is `{part} dropped an SPI transaction from #{sender} whose data lanes are not on its own data pins at {t_ps} ps, the first time`.

`data` holds `sender`, the part name of the chip that sent it. A transaction goes to every chip whose `CS` and `SCLK` are on the sender's. A lane it uses, up to the most lanes any of its phases or its data take, must be on the receiver's pin of the same number, `D0` to `D3`. Otherwise the receiver drops it, and the run reports the first such drop once per part.

## chip.link_unsupported

Severity `error`. Refs are the sender's pin, then the pin on the same net that does not take that record kind. `t_ps` is the end of the first such record. It ends the run.

The message depends on `kind`. For `spi`, it is `{pin} sent an SPI record from {peripheral} at {t_ps} ps, but its net reaches {other}, which does not take SPI records there; the model sends {peripheral} output as records rather than pin edges and runs hardware SPI only into parts that take SPI directly, so use shiftOut or bit-banged SPI instead`. For `pulses`, it is `{pin} sent a pulse record from {peripheral} at {t_ps} ps, but its net reaches {other}, which does not take pulse records there; the model sends {peripheral} output as records rather than pin edges, so the data line must be wired straight to the pixel's data input`.

`data` holds `kind` (`"spi"` or `"pulses"`) and `peripheral`, the peripheral that sent the record. A `Pulses` or `Spi` record carries no pin edges, so an active net may reach only the sender's pin, parts that take that record kind on that pin, and pull-up or pull-down resistors to ground or a fixed supply rail. Any other element on the net, such as a series resistor, a transistor, a level shifter, another chip input that reads pin edges, or another GPIO pin, stops the run at the first record. `Duty` records also drive pin edges and do not raise this diagnostic.

## chip.floating_input

Severity `warning`. Refs are the chip's pin, then `#net.<name>`, the pin's net. `t_ps` is the first accepted step that found it.

The message is:

```text
{pin} floats on {net} at {t_ps} ps, and the firmware has taken in its level since the pin last became an input with its pull-up off, alone or with the rest of its port: there is no DC path to a supply or to ground, and a capacitor does not hold the pin's level, so the level is whatever leakage and noise leave; add a pull-up or a pull-down resistor, or turn the pin's pull-up on
```

`data` is `{}`. The run reports each pin once, at the first accepted step where all three hold:

- The pin's drive is off: an input with its pull-up off, and with no pull-down.
- Its firmware has read the pin at or before that step, since its drive last turned off. A read while the pin drives or pulls up takes in a level the chip holds, so it does not count, and neither does a read from before the pull-up last came and went. An ATmega328P counts a read of a pin when an instruction loads its port's `PINx` while the pin's drive is off and its digital input is on, and for as long as an enabled `INTn` or pin-change interrupt watches it with its drive off. A pin whose digital input is off, `PC6` while it is `RESET` or a pin `DIDR0` turns off, is not read. A chip on the [contract](mcu.md) that says nothing reads no pin, so the check covers the ATmega328P alone. The engine asks the copy of the core it runs ahead, which has always reached the step, since a landing commits only a core that stops there. That copy is past no change of its drive but the one at its stop, so while the committed drive is off at the step, the copy's stretch with the drive off is the one that holds the step.
- The pin's net reaches ground through no DC path but chips' leakage. The paths are those of [`net.floating`](#netfloating), less every active chip's and every capacitor's. A switch joins its pins only while its contact is closed on that step, and an active chip joins an output's net to its own supply while it drives high or pulls up, and to its own ground while it drives low or pulls down.

A load of `PINx` takes in all eight pins of the port, and the chip cannot tell which of them the firmware keeps: `digitalRead` loads the whole byte and masks its bit in software, with a mask it looks up at run time. So a load counts for every pin of the port whose drive is off then, and the message says the firmware took the level in with the rest of its port rather than that it read that pin. That level is in the byte the firmware holds, and DS40002061B §14.2.6 asks for a defined level on every unused pin, since a floating input draws current in every mode where the digital inputs are on. A pin `D4` with only a capacitor to ground, on a Nano whose sketch reads only `D2`, warns once the sketch first reads `D2`.

A static check cannot tell this: a sketch sets `INPUT` or `INPUT_PULLUP` as it runs, and a pin the sketch never reads may float harmlessly. A net that holds only the pin, with no other part, is not checked. A button from the pin to 5V with no pull-down warns at the first read while the button is open. With a pull-down, an external pull-up, `INPUT_PULLUP`, or a pin the firmware never reads, it does not. A sketch that calls `digitalRead` under `INPUT_PULLUP` and then sets `INPUT` without reading again does not warn. One that reads first and sets `INPUT_PULLUP` after warns at the step where the pull-up comes on.

## chip.unsupported_instruction

Severity `error`. Refs are the chip. `t_ps` is the time the core reached the instruction. It ends the run.

The message is `{part} reached {what} at {pc}, which the model does not run, at {t_ps} ps`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `op`, `pc`, `reason` and `symbol`. `op` is the op's name. `pc` is its address. `symbol` is the ELF symbol at or below the PC, written `name+0x1c` past its start, or `null`. When the APP CPU ran the code, the message has ` on the APP CPU` after the PC and its symbol, and `data` also holds `core`, `"app"`. Every `esp32s3` diagnostic with a `pc` writes the core this way. `reason` is one of these.

- `op`: the core does not model the op.
- `unaligned`: the op is an `l32ai`, an `s32ri` or an `s32c1i` at an address that is not a multiple of 4, or another load or store whose access is unaligned and one of whose parts faults.
- `litbase`: an `l32r` while `LITBASE` is enabled.
- `loop_end`: the op is the last of a zero-overhead loop and loops back, and it is a call, `isync`, `waiti` or `rsr.lcount`, which the ISA rules out there.

On an `atmega328p`, `op` is `unknown` (a flash word that decodes to no instruction) or `spm` (an `SPM` instruction executed from the Boot Loader Flash section), and `reason` is `op`. For `unknown`, the message reads `a word that decodes to no instruction` for `{what}`. `pc` is the word's byte address. An erased flash is all `0xFFFF` words, so a firmware that runs off its end stops at the first one.

## chip.unsupported_setting

Severity `error`. Refs are the chip. `t_ps` is the time of the store that started the transfer or set the setting. It ends the run.

The message is `{part} started a {peripheral} transfer with setting {setting} at {pc}, which the model does not run, at {t_ps} ps` for a transfer, `{part} enabled a {peripheral} interrupt at {pc}, which the model never raises, at {t_ps} ps` for an interrupt, `{part} set {peripheral} {setting} at {pc}, which the model does not run, at {t_ps} ps` for a register setting, with `; ESP-IDF sets GDMA's reset when it restarts or panics, so the console holds any panic text` after `does not run` for `GDMA` `reset_while_sending`, and `{part} put the chip to sleep at {pc}, which the model does not run, at {t_ps} ps` for sleep, each with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `peripheral`, `pc`, `setting` and `symbol`. `pc` is the address of the store, and `symbol` is written as `chip.unsupported_instruction` writes it. For a transfer, `peripheral` is `SPI2` or `SPI3`, and `setting` is the first of these the registers held when the store wrote `SPI_USR`: `slave`, `dma`, `dma_link`, `dma_descriptor`, `dma_length`, `lsb_first`, `qpi`, `octal`, `dtr`, `dummy`, `read`, `high_part`, `three_line`, `full_duplex`, `clock_mode`, `cs_disabled`, `data_length`, `clock_off` or `unsynced`. `setting` is `interrupt` when the store set an interrupt enable bit of a source the model never raises, with `peripheral` `SPI2`, `SPI3`, `GDMA`, `TIMG0`, `TIMG1`, `USB_SERIAL_JTAG`, `SHA`, `AES`, `RSA`, `LEDC`, `PCNT`, `RMT`, `UART0`, `UART1` or `UART2`, or with `peripheral` `SENS` when the store set `RTC_CNTL_SARADC1_INT_ENA`, `RTC_CNTL_TSENS_INT_ENA` or `RTC_CNTL_SARADC2_INT_ENA`, bits 11, 12 and 14 of `RTC_CNTL_INT_ENA_RTC_REG`. It is `sleep`, with `peripheral` `RTC_CNTL`, when the store set `RTC_CNTL_SLEEP_EN`. These register settings end the run too:

- `auto_wait_idle`, `auto_suspend`, `suspend` and `interrupt`, with `peripheral` `SPI1`: the store set `SPI_MEM_WAITI_EN` in `SPI_MEM_FLASH_WAITI_CTRL_REG`, `SPI_MEM_FLASH_PES_EN` in `SPI_MEM_FLASH_SUS_CTRL_REG`, a suspend or resume bit in `SPI_MEM_FLASH_SUS_CMD_REG`, or any bit in `SPI_MEM_INT_ENA_REG`.
- `memory_copy`, `write_back` and `restart`, with `peripheral` `GDMA`: the store set `GDMA_MEM_TRANS_EN`, `GDMA_OUT_AUTO_WRBACK`, `GDMA_INLINK_RESTART` or `GDMA_OUTLINK_RESTART`.
- `link_peripheral`, with `peripheral` `GDMA`: the store started an OUT link whose `GDMA_PERI_OUT_SEL` names none of SPI2, SPI3, AES or SHA, or an IN link at a descriptor in SRAM whose `GDMA_PERI_IN_SEL` is none of SPI2, SPI3 or AES.
- `peri_sel_while_started`, with `peripheral` `GDMA`: the store changed a channel's `GDMA_PERI_OUT_SEL` or `GDMA_PERI_IN_SEL` while its link was started.
- `buffer_write`, with `peripheral` `GDMA`: the store reached a byte of a descriptor or a buffer that a running SPI2 or SPI3 transfer sends from. A byte that only shares a word with one does not count.
- `mode`, `t_length`, `dma_link`, `dma_descriptor` and `dma_length`, with `peripheral` `SHA`: the store started a hash in an unsupported mode or with an invalid `SHA_T_LENGTH_REG`, or a DMA hash whose GDMA channel or list GDMA refused with that setting. `dma_link` also covers a start while GDMA's clock is off.
- `mode`, `endian`, `gcm`, `block_mode`, `block_num`, `dma_link`, `dma_descriptor` and `dma_length`, with `peripheral` `AES`: the store wrote a reserved `AES_MODE_REG` or `AES_BLOCK_MODE_REG`, a non-zero `AES_ENDIAN_REG`, `AES_CONTINUE_REG` (GCM), a zero `AES_BLOCK_NUM_REG` on DMA start, or a DMA-AES start whose GDMA channel or list GDMA refused.
- `power_down`, `not_clean`, `length` and `power_down_while_running`, with `peripheral` `RSA`: the store started an RSA operation while memory was powered down or before `RSA_QUERY_CLEAN_REG` set, or with an invalid `RSA_LENGTH_REG`, or set `SYSTEM_RSA_PD_CTRL_REG` to power down memory while an operation ran.
- `reset_while_sending` and `clock_off_while_sending`, with `peripheral` `SPI2`, `SPI3` or `RMT`: the store set that peripheral's reset bit, or cleared its clock bit, while an SPI2 or SPI3 transfer or an RMT transmission ran. With `peripheral` `GDMA`, the store did that to GDMA's bits while an SPI2, SPI3 or DMA-AES transfer used it. ESP-IDF's restart and panic paths set GDMA's reset, `SYSTEM_DMA_RST`, in `esp_system_reset_modules_on_exit`, so an `esp_restart()` or a panic while SPI2, SPI3 or DMA-AES uses GDMA ends the run here, at a PC in that function, in place of `chip.restart` or `chip.halted`. The console holds a panic's text, which ESP-IDF prints first.
- `reset_while_running`, `clock_off_while_running` and `clock_change_while_running`, with `peripheral` `AES` or `RSA`: the store set the peripheral's reset bit, cleared its clock bit, or changed the APB clock rate while an operation ran.
- `hw_flow`, `sw_flow`, `sw_rts`, `sw_dtr`, `rs485`, `irda`, `autobaud`, `break`, `power_down`, `glitch`, `stop_bits`, `fifo_size`, `high_speed`, `divider`, `clock`, `sclk_frac`, `clock_off`, `mem_clock`, `reset_while_sending`, `baud_while_sending` and `clock_change_while_sending`, with `peripheral` `UART0`, `UART1` or `UART2`: the store set hardware or software flow control (except `UART_SW_FLOW_CON_EN` with `UART_FORCE_XOFF` alone, which ESP-IDF's light sleep writes before `RTC_CNTL_SLEEP_EN`, unless a TX FIFO write follows), set `sw_rts` or `sw_dtr`, RS485, IrDA (CONF0 bits 9 to 13 and bit 16; loopback, bit 14, is run), autobaud, a break or `mem_force_pd`, enabled the glitch filter, wrote a stop-bit field of 0, wrote an `rx_size` or `tx_size` other than 1, cleared `high_speed`, would start a frame at a divider of 0, selected a baud clock other than APB or the crystal, wrote a nonzero `sclk_div_a` or `sclk_div_b`, wrote the TX FIFO while the baud clock was off and the module clock still ran, read or wrote the FIFO or would start a frame while the module was running and `mem_clk_en` or `SYSTEM_UART_MEM_CLK_EN` was clear, reset a FIFO or the baud core while a frame was shifting, changed the divider, the frame, the idle count, an inversion, loopback or the baud-clock divider or source while a frame was shifting, or changed the APB rate while an APB-clocked port was shifting. A module clock that is off, or a peripheral reset that holds the port, does not end the run. The access completes and the shifter stops, as [esp32s3.md](esp32s3.md#what-it-models) says.
- `fifo_pointers`, with `peripheral` `UART0`, `UART1` or `UART2`: the store wrote the TX FIFO, or the load read the RX FIFO, after a peripheral or core reset of that port that came after a byte entered that FIFO, with no reset bit for that FIFO since: `txfifo_rst` for the TX FIFO, `rxfifo_rst` for the RX FIFO. A reset keeps the FIFO pointers and empties the FIFO, as a board capture on 2026-11-18 showed for UART0's TX side, so the model cannot say where the byte lands or which byte the shifter or the load takes. [esp32s3.md](esp32s3.md#what-it-models) gives the rule.
- `clock_off` and `held_in_reset`, with `peripheral` `SPI2`, `SPI3`, `LEDC`, `GDMA`, `SHA`, `AES`, `RSA`, `I2C0`, `I2C1`, `RMT` or `PCNT`: a load or a store reached that peripheral, or `RMTMEM` for `RMT`, while its clock gate was clear, or while a reset bit held it. `pc` is that access's.
- `channel`, `dma`, `sync`, `rx`, `stop_while_sending` and `start_while_sending`, with `peripheral` `RMT`: a store to `CHnCONF0` for n from 1 to 3 sets `TX_START` or `IDLE_OUT_EN`, a store to `CHnCONF0` for n from 0 to 3 sets `DMA_ACCESS_EN`, a store sets `TX_SIM_EN`, a store to `CHmCONF1` for m from 4 to 7 sets `RX_EN`, or a store sets `TX_STOP` or `TX_START` on channel 0 while it sends. `TX_STOP` is named before `TX_START`.
- `fifo`, `power_down`, `clock_source`, `sclk_active`, `fractional_divider`, `mem_size`, `carrier`, `loop`, `no_wrap`, `idle_out` and `inverted`, with `peripheral` `RMT`: `TX_START` on channel 0 finds `APB_FIFO_MASK` clear, `MEM_FORCE_PD` set, `SCLK_SEL` other than 1, `SCLK_ACTIVE` clear, `SCLK_DIV_A` other than 0, `MEM_SIZE` other than 1, `CARRIER_EN` set, `TX_CONTI_MODE` set, `MEM_TX_WRAP_EN` clear, `IDLE_OUT_EN` clear, or `OUT_INV_SEL` set on a pad that carries signal 81. A store to `CHnDATA`, offsets `0x00` to `0x1c`, is `fifo` as well. The store writes nothing.
- `clock_change_while_sending` and `no_end`, with `peripheral` `RMT`: a store would change the APB rate while channel 0 sends, and it writes nothing, or a transmission reaches 65,536 entries with no end word. For `no_end`, `pc` is the instruction after the store that started the transmission.
- `app_cpu_without_reset`, with `peripheral` `SYSTEM`: a store made the APP CPU runnable and it has not been reset in this run, so the model cannot know its state. The run ends at a later store to `SYSTEM_CORE_1_CONTROL_0_REG` that does not set `RESETING`, or when 1,000 PRO CPU cycles pass with no reset. `pc` is that store, or the instruction the PRO CPU had reached at the bound, which need not be a store. The 1,000 cycles are the model's bound, not a chip figure. ESP-IDF's `cpu_utility_ll_enable_clock_and_reset_app_cpu` sets `CLKGATE_EN`, clears `RUNSTALL`, then sets and clears `RESETING`, and the APP CPU starts at that clear, as [esp32s3.md](esp32s3.md#the-two-cores) says.
- `pro_cpu_stall`, with `peripheral` `RTC_CNTL`: the store left `RTC_CNTL_SW_STALL_PROCPU_C0` at 2 and `RTC_CNTL_SW_STALL_PROCPU_C1` at `0x21`, which would stall the PRO CPU. The model does not stall it.
- `watchpoint`, with `peripheral` `CPU`: a load or a store reached a byte of a window `DBREAKAn` and `DBREAKCn` name. `pc` is that instruction, and the access does not complete. `DBREAKCn` bits 0 to 5 select which low address bits are compared, a set bit compared and a clear bit ignored. Bit 30 arms a load and bit 31 a store.
- `hpoint` and `duty_fraction`, with `peripheral` `LEDC`: the store sent a channel's update with a nonzero `LEDC_HPOINT_LSCHn` or with a duty that has a fraction.
- `clock`, `tick_sel`, `clock_divider`, `duty_resolution`, `fractional_divider`, `duty_range` and `full_duty`, with `peripheral` `LEDC`: an update waits on a counting timer whose `LEDC_APB_CLK_SEL` is neither 1 nor 3, whose `LEDC_TICK_SEL_LSTIMERn` is set, whose divider is below 256, whose resolution is 0 or above 14, or whose period is not whole ticks of 480 MHz, or the update's duty or fade target is at or above `2^14` or below 0, or is `2^14` at 14 bits; or a counting timer latches such a setting through `LEDC_LSTIMERn_PARA_UP` or sets `LEDC_TICK_SEL_LSTIMERn`.
- `clock_change_while_running`, with `peripheral` `LEDC`: the store changed `LEDC_APB_CLK_SEL` or the APB clock while a timer counted it.
- `clock_off_while_running`, with `peripheral` `LEDC`: the store cleared LEDC's clock gate while a timer counted.
- `reset_while_running`, with `peripheral` `LEDC`: the store set LEDC's reset bit while a pin with a net carried a channel through the matrix, or while an update had yet to take effect or to drive its signal.
- `watchdog_cpu_reset`, `watchdog_interrupt` and `watchdog_stage`, with `peripheral` `TIMG0`, `TIMG1` or `RTC_CNTL`: the store left a running watchdog, at or before its first stage that resets, a CPU reset while its `PROCPU_RESET_EN` is clear, an interrupt of the RTC watchdog, or a stage field that names no action.
- `super_watchdog`, with `peripheral` `RTC_CNTL`: the store left both `SWD_AUTO_FEED_EN` and `SWD_DISABLE` clear in `RTC_CNTL_SWD_CONF_REG`.
- `pin_int_ena`, `pin_int_type` and `pin_wakeup`, with `peripheral` `GPIO`: the store set a `GPIO_PINn` word whose `INT_ENA` enables the NMI or an SDIO interrupt, whose `INT_TYPE` is the reserved 6 or 7, or whose `WAKEUP_ENABLE` is set.
- `pull_down`, with `peripheral` `IO_MUX`: the store set `FUN_WPD` on a pad with a net, and the part names no `pull_down_resistance`. The model has no resistor to stamp. A pad with no net keeps the bit, because it is never stamped.
- `clock_off`, with `peripheral` `RNG`: a load reached `WDEV_RND_REG` while `SYSTEM_WIFI_CLK_RNG_EN` was clear. No source says what the register reads then.
- `phy_sel`, `exchg_pins`, `vref`, `pad_pull`, `pullup_value`, `phy_tx_edge` and `jtag_bridge`, with `peripheral` `USB_SERIAL_JTAG`: a store to `USB_SERIAL_JTAG_CONF0_REG` set `PHY_SEL`, `EXCHG_PINS_OVERRIDE` or `EXCHG_PINS`, `VREF_OVERRIDE`, `VREFH` or `VREFL`, `DP_PULLDOWN`, `DM_PULLUP` or `DM_PULLDOWN`, `PULLUP_VALUE`, `PHY_TX_EDGE_SEL`, or `USB_JTAG_BRIDGE_EN`.
- `unsimulated`, with `peripheral` the block: a load or a store reached a register no model keeps in a block the model does not simulate, such as `MCPWM0`, `I2S0`, `TWAI`, `USB_WRAP`, `BT` or `RADIO`, as [esp32s3.md](esp32s3.md#unmodeled-registers) lists. The message reads `{part} used {peripheral}, {what}, at {pc}, which the model does not simulate, at {t_ps} ps`, where `{what}` names the block for a maker, such as `the motor-control PWM that ESP32Servo drives servos with`. `pc` is that access's.
- `slave_mode`, `push_pull`, `not_routed`, `lsb_first`, `arbitration`, `sample_scl_level`, `clock_off` and `timing`, with `peripheral` `I2C0` or `I2C1`: `I2C_TRANS_START` left `MS_MODE` clear, left `SDA_FORCE_OUT` or `SCL_FORCE_OUT` clear or a routed pad without `PAD_DRIVER`, left signals 89 and 90 (I2C0) or 91 and 92 (I2C1) unrouted both ways, set `TX_LSB_FIRST` or `RX_LSB_FIRST`, set `ARBITRATION_EN`, set `SAMPLE_SCL_LEVEL`, left `SCLK_ACTIVE` clear, or copied a timing whose SDA hold is at least the SCL low period or whose SDA sample is at least `I2C_SCL_HIGH_PERIOD`. `I2C_CLK_EN` is kept and is not this `clock_off`.
- `rc_fast_clock`, `fractional_divider`, `nonfifo`, `fsm_reset`, `clear_bus`, `power_down` and `int_ena`, with `peripheral` `I2C0` or `I2C1`: the store set `SCLK_SEL`, a nonzero `SCLK_DIV_A` or `SCLK_DIV_B`, `NONFIFO_EN`, `FSM_RST`, `SCL_RST_SLV_EN`, `SCL_PD_EN` or `SDA_PD_EN`, or an `INT_ENA` bit outside the NACK, timeout, transfer-complete, arbitration-lost and end-detect bits.
- `opcode`, with `peripheral` `I2C0` or `I2C1`: a command is op 0, 5 or 7, or a read or a write whose byte count is 0, or the first command on a bus no END holds is neither a repeated START nor a STOP.
- `tx_underflow`, `tx_overflow` and `rx_overflow`, with `peripheral` `I2C0` or `I2C1`: a write command finds the TX FIFO short, a store to `I2C_DATA_REG` finds it full, or a read command finds the RX FIFO full.
- `rx_full_ack_level`, with `peripheral` `I2C0` or `I2C1`: a read command's byte fills the 32-byte RX FIFO, and the command's ACK is not the level `I2C_RX_FULL_ACK_LEVEL` sets.
- `busy_write`, with `peripheral` `I2C0` or `I2C1`: a store of `TRANS_START`, `CONF_UPGATE`, a command, a FIFO reset or a timing word came while a transfer ran, except command writes while a NACK's automatic stop runs and no-op `CONF_UPGATE` or `TRANS_START` strobes with `COMD0` holding `STOP` while a NACK's automatic stop or a standalone `STOP` started right after a NACK runs.
- `glitch`, with `peripheral` `I2C0` or `I2C1`: with the filter on, SCL or SDA held a level for fewer sclk cycles than its threshold while a transfer ran.
- `reset_while_sending`, with `peripheral` `I2C0` or `I2C1`: a gate store cleared that port's clock bit or set its reset bit while a transfer ran.
- `data_inv`, `sample_num`, `clk_gated`, `wait_arb_cycle`, `amplifier`, `pwdet`, `dig_controller`, `rtc_force` and `sarclk_en`, with `peripheral` `SENS`: the store set `SARn_DATA_INV` or a nonzero `SARn_SAMPLE_NUM`, cleared `SARn_CLK_GATED`, or left `SAR2_WAIT_ARB_CYCLE` other than 1, in `SENS_SAR_READER1_CTRL_REG` or `SENS_SAR_READER2_CTRL_REG`; changed `SENS_SAR_MEAS1_CTRL1_REG` or one of `SENS_SAR_AMP_CTRL1_REG` to `SENS_SAR_AMP_CTRL3_REG` from its reset value; changed `SENS_SAR_MEAS2_CTRL1_REG` from its reset value outside `SAR2_CNTL_STATE`, its three read-only bits; set `SENS_SAR1_DIG_FORCE` or `SENS_SAR2_RTC_FORCE`; or set `SENS_SARCLK_EN`.
- `change_while_converting`, with `peripheral` `SENS`: the store changed a unit's reader, `MEASn_CTRL2`, a start among those changes, `MEASn_MUX` or `SENS_SAR_ATTENn_REG` word while that unit converted, or changed `FORCE_XPD_SAR`, `SARADC_CLK_EN` or `SARADC_RESET` while either did.
- `h_lim`, `l_lim`, `ctrl`, `clock_off_while_running` and `clock_change_while_running`, with `peripheral` `PCNT`: the store wrote a negative `CNT_H_LIM`, a positive `CNT_L_LIM`, reserved bits in `PCNT_CTRL_REG`, cleared `SYSTEM_PCNT_CLK_EN` while a glitch-filter timer was pending, or changed the APB rate while a glitch-filter timer was pending.
- `tsens_in_inv`, `tsens_clk_inv`, `tsens_clock_off`, `tsens_held_in_reset`, `tsens_power`, `tsens_clk_div` and `tsens_range`, with `peripheral` `SENS`: the store set `SENS_TSENS_IN_INV`, cleared `SENS_TSENS_CLK_INV`, or raised `SENS_TSENS_DUMP_OUT` while `SENS_TSENS_CLK_EN` or `RTC_CNTL_DIG_CLK8M_EN` was clear, `SENS_TSENS_RESET` was set, the sensor power force bits were off, `SENS_TSENS_CLK_DIV` was 0, or `I2C_SARADC_TSENS_DAC` held an undocumented range code.
- `ulp_controller`, `channels`, `pad_no_net`, `power`, `clock_off`, `held_in_reset`, `clk_div`, `arbiter`, `calibration`, `initial_code`, `dref`, `sample_cycle`, `internal_ground` and `test_mux`, with `peripheral` `SENS`: the store raised `MEASn_START_SAR` and the first of these held. `MEASn_START_FORCE` or `SARn_EN_PAD_FORCE` was clear, so the ULP would drive the start. `SARn_EN_PAD` named no channel, more than one, or one above 9. The channel's GPIO, `n + 1` on ADC1 and `n + 11` on ADC2, has no net. `FORCE_XPD_SAR` was not 3. `SARADC_CLK_EN` was clear. `SARADC_RESET` was set. `SARn_CLK_DIV` was 0. ADC2's arbiter had `ADC_ARB_GRANT_FORCE` set and `ADC_ARB_RTC_FORCE` clear. The chip's eFuse set gives IDF no calibration, or a `digi` of 0, as [esp32s3.md](esp32s3.md#the-sar-adcs) says. The unit's initial code in the analog I2C was not the one IDF works out from the set for the channel's attenuation. `DREF` was not 4. ADC1's `SAMPLE_CYCLE` was not 0. The unit's `ENCAL_GND` was set. A bit of the analog test mux, bits 0 to 4 of the SAR block's register 7, was set.

The access writes nothing, apart from an `app_cpu_without_reset` or `pro_cpu_stall` store, whose word keeps the value it stored. An `app_cpu_without_reset` at the 1,000-cycle bound is not a store. The run keeps every row before it, as [esp32s3.md](esp32s3.md#what-it-models) says.

An `atmega328p` ends the run on these, each a firmware store unless it says otherwise. `pc` is the instruction's byte address. The store completes, and nothing the chip does after it shows. [avr.md](avr.md#not-yet-modelled) gives each one's datasheet reason.

- `aco_read`, with `peripheral` `AC`: a load of `ACSR` while `ACD` is clear. The message reads `read the AC output while the comparator is on` and ends `which the model works out from the logic level on each input pin, not its voltage`. A store that sets `ACD` within 1024 CPU cycles of the load shows it was `ACSR |= _BV(ACD)`, and clears it. A second load, a store that leaves `ACD` clear, or the 1024 cycles passing end the run at the load. The 1024 cycles are the model's bound, not a chip figure.
- `ACIE`, `ACIC` and `ACBG`, with `peripheral` `AC`: the store left `ACD` clear and set that bit.
- `ADTS`, with `peripheral` `ADC`: the store wrote `ADTS` 001, the comparator trigger, while `ACD` was clear.
- `REFS` and `MUX`, with `peripheral` `ADC`: the store wrote `REFS` 10, or `MUX` 1001 to 1101.
- `IVSEL`, with `peripheral` `CPU`: the store set `MCUCR.IVSEL`.
- `SPMCSR`, with `peripheral` `BOOT_LOAD`: the store set a bit of `SPMCSR` below `SPMIE`. With `SPMIE` set, `setting` is `interrupt`.
- `AS2` and `EXCLK`, with `peripheral` `TC2`: the store set that bit of `ASSR`.
- `synchronous`, `UMSEL` and `UPM`, with `peripheral` `USART0`: the store wrote `UMSEL0` 01 or 10, or `UPM0` 01, to `UCSR0C`.
- `slave`, with `peripheral` `TWI`: a store to `TWAR`, or a store setting a nonzero bit 7:1 mask in `TWAMR`, while `TWCR` holds `TWEN` and `TWEA`, or a store to `TWCR` setting both after such a store since the last reset.
- `sleep`, with `peripheral` `CPU`: a `SLEEP` with `SE` set in `SM` 100 or 101. The message reads `put the chip to sleep in a reserved sleep mode`, and `pc` is the `SLEEP`'s.
- `unsimulated`, with `peripheral` `Optiboot`: either a byte reached USART0 while Optiboot waited for an upload after a reset from `RESET`, on a part that names `"bootloader": "optiboot"` with `BOOTRST` programmed (`pc` is `getch`, `0x00007fa6`, and `t_ps` is the poll that found `RXC0` set), or the sketch jumped or called into Optiboot's unloaded boot section (`0x7E00` to `0x7FFF`, such as `do_spm` at `0x7FFC`), where `pc` is the fetched boot-section word and `{what}` is `its boot-section code (such as do_spm)`. Neither is a store.

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

- `unsupported`: a command the model does not know. The message reads `sent the flash command 0xb9`, `which the model does not run`.
- `status_write`: the fixed `WRSR`, or a user `0x01`, `0x31` or `0x11`, with or without `WREN`. A status write can set the flash's protect and one-time lock bits. The message reads `sent the flash the status write command 0x01`, `which could lock the flash`.
- `no_command_phase`: a user command with no command phase, or a write that sets only `SPI_MEM_FLASH_PE`. `command` is `null`.
- `start_bits`: one write of `SPI_MEM_CMD_REG` that starts more than one command, such as `WREN` with a sector erase. None of them runs. `command` is `null`.
- `past_end`: a read, page program or erase that reaches past the flash's end. A chip with no flash refuses every read, program and erase. The message reads `sent the flash command 0x03`, `which reaches past the flash's end`.
- `address_bits`: a command whose address phase is longer than 32 bits, or does not match what the command requires.
- `busy`: an SPI1 command other than a status read, or a cache fetch or load through the MMU (`command` is `null`), while a program or erase is still in progress (`WIP` set).
- `length`: a command with an invalid phase or byte count, such as a page program with zero bytes or more than 64 bytes in one SPI1 transaction.
- `mode`: an SPI1 command started in octal, DDR, or an unsupported write-width mode.
- `spi0`: a write of SPI0's `SPI_MEM_CMD_REG` that starts a command. `command` is `null`.
- `pads`: a write that would take one of the flash's pads, GPIO26 to GPIO32, from SPI0 and SPI1. It gives the pad an IO_MUX function other than 0, sets its `GPIO_ENABLE` bit, or moves its `GPIO_FUNCn_OUT_SEL_CFG_REG` from its reset value, as [esp32s3.md](esp32s3.md#what-it-models) says. `command` is `null`. The message reads `took a flash pad from SPI0 and SPI1`, `which could send the flash any command`.
- `cs`: a user command while `SPI_MEM_MISC_REG` has `CS0_DIS` set or `CS1_DIS` clear. The register resets to `0x2`, so CS0 is on and CS1 is off. The fixed commands do not look at it. The message reads `sent the flash command 0x03 with CS0 disabled or CS1 enabled`, `which the model does not run`.

The flash reads, programs, erases, and reports its ID and status, as [esp32s3.md](esp32s3.md#what-it-models) says.

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

## chip.power_on_restart

Severity `error`. Refs are the chip. `t_ps` is 1 ps after the supply returned to the chip's operating range following a brown-out. It ends the run.

The message is `{part} supply returned at {t_ps} ps; power-on restart of the ESP32-S3 is not simulated yet`.

`data` is `{}`. When an `esp32s3` part's `V(VCC) - V(GND)` falls to or below 2.44 V (ESP-IDF v5.5.5 `CONFIG_ESP_BROWNOUT_DET_LVL_SEL_7`, the default brown-out detector level; ESP32-S3 Series Datasheet v2.2 Section 2.5.3), the chip stops executing instructions, stops all peripherals and releases every GPIO pad to off (leaving only pad leakage). When `V(VCC) - V(GND)` rises back to 3.0 V (ESP32-S3 Series Datasheet v2.2 Section 5.2, Table 5-2 minimum recommended operating `VDD`) after a power loss, the run stops here because booting again from power-on is not simulated yet. A chip that starts unpowered at t = 0 and powers on once does not raise this error.

## chip.unmapped_load

Severity `error`. Refs are the chip. `t_ps` is the time of the load. It ends the run.

The message is `{part} loaded from {address} at {pc}, where the MMU maps no flash, at {t_ps} ps`, with ` in {symbol}` after the PC when the firmware has a symbol at or below it.

`data` holds `address`, `pc` and `symbol`. `address` is the address the load read, in the instruction or data cache window. `pc` and `symbol` are as `chip.unsupported_instruction` writes them. The MMU entry of that page is invalid, or picks PSRAM, which the chip lacks, or maps a page past the flash's end, or the chip has no flash, as [esp32s3.md](esp32s3.md#what-it-models) says.

## chip.unmodeled_register

Severity `warning`. Refs are the chip. `t_ps` is the time of the first access to the register, or for a register the model keeps without running its effect, the time of that write.

The message is `{part} read {register} at {address}, which the model does not keep, first at {t_ps} ps from {pc}`, with `wrote` for a write and ` in {symbol}` after the PC when the firmware has a symbol at or below it. For a register the model keeps without running its effect, it reads `{part} wrote {register} at {address}, whose effect the model does not run, at {t_ps} ps from {pc}`.

`data` holds `address`, `kept`, `pc`, `register`, `symbol` and `write`. `address` is the register's word address. `register` names it by its block and the offset into the block, such as `RTCIO+0x88` or `TIMG0+0x24`. `pc` is the address of the instruction that made the access, and `symbol` is written as `chip.unsupported_instruction` writes it. `write` is true for a store. A read of such a register gives 0 and a store is dropped. The run reports each register once per chip, at its first access, whatever later accesses do. A register the chip accepts quietly, named in [esp32s3.md](esp32s3.md#unmodeled-registers), is not reported. Its read is still 0 and its store is still dropped. The registers whose writes change the slow clock or SPI0's read command, and the clock gate and reset bits of the peripherals the model keeps without running, are kept, and their effect does not run. `kept` is true for those and false for every other register. The run reports each write that changes one of those, every time, as [esp32s3.md](esp32s3.md#what-it-models) lists. For the peripherals' gate and reset words that is a write that changes a clock bit of SYSTIMER, TIMG0, TIMG1, SPI0, SPI1, the eFuse controller, the USB Serial/JTAG unit, `SYSTEM_TIMERS` or `SYSTEM_WDG`, or a reset bit of SPI0, SPI1, the eFuse controller, the USB Serial/JTAG unit or `SYSTEM_TIMERS`. A reset of SYSTIMER, a timer group or `SYSTEM_WDG` runs, so that write is not listed. Such a write is listed once, when it lands, so a write that the core never runs again after it stops there is not listed. The list is the work queue of registers a firmware needs, as [esp32s3.md](esp32s3.md#unmodeled-registers) says.

An `atmega328p` keeps every register, so each of its notes has `kept` true and `write` true, and the run lists each once, at the first store that makes it. `address` is the register's data address, and `register` is its datasheet name. [avr.md](avr.md#not-yet-modelled) gives each one's datasheet reason.

- `TWBR`, at `0xb8`: a store to `TWCR` that started a START at a bit rate whose data hold time passes Table 29-14's maximum, below about 18 kHz. The store is to `TWCR`, and the note names the register that sets the rate.
- `OSCCAL`, at `0x66`: any store.

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

Severity `warning`. Refs is the rated quantity, `#<part>.i` (a voltage source's too), `#<part>.ic` for a bipolar transistor, `#<part>.id` for an `nmos` or a `pmos`, `#<part>.iout` for a linear regulator, or a chip pin, `#<part>.pin.<pin>`, for an `atmega328p`. `t_ps` is the first time over the rating, 0 for the operating point.

The message is one of:

```text
{ref} peaks at {peak} A, over its {limit} A rating for {over_ps} ps
{ref} carries {peak} A RMS with a {tau} ms time constant while it is {sample} A, over its {limit} A rating for {over_ps} ps
{ref} averages {peak} A with a {tau} ms time constant while it is {sample} A, over its {limit} A rating for {over_ps} ps
{ref} is {sample} A now, over its {limit} A rating; it warns if held about {held_ps} ps
```

The first is a peak rating's, or a sustained rating's `peak` limit. The second and third are a [sustained rating](#sustained-ratings)'s, RMS or mean as its basis says. The fourth is a sustained rating [over now](#over-now). When the rating sets `"characterized": true`, the phrase reads `beyond the {limit} A load its sheet characterizes` instead of `over its {limit} A rating`, and `data` includes `"characterized": true`. Each value in a message is rounded to 3 significant digits, `tau` too, and `held_ps` to 2. ` while it is {sample} A` is left out when it prints the same as the reading, as at an operating point. ` for {over_ps} ps` is left out for an operating point, and while `over_ps` is 0, as on the step where the quantity first goes over. A session's `diagnostics()` writes each message as it stands through the last accepted step, so a warning seen early reads a shorter time, or none, than the run folder's. `over_ratings` writes it through its probe, so a live session's `snapshot` reads the time up to `now`.

`data` holds `average`, `held_ps`, `limit`, `now`, `over_ps`, `peak`, `peak_ps`, `sample` and `tau_ps`, unrounded. `limit` is the rating. `peak` is the largest magnitude of the quantity, or the largest reading of a sustained rating's filter. `peak_ps` is when it peaked. `sample` is the quantity's own magnitude at `peak_ps`, which is `peak` for a peak rating. `over_ps` is `null` for an operating point, which has no duration. `average` is `"rms"` or `"mean"` and `tau_ps` the time constant for a sustained rating, and both are `null` for a peak rating. `now` is `false` and `held_ps` is `null`, except for a rating over now.

A transient checks every accepted step, not only the rows. Each step's value holds until the next accepted step. `t_ps` is the first step over the rating, and `over_ps` adds the time to the next step for every step over it. The last step adds nothing. So `t_ps` and `peak_ps` can fall between rows. At a switch edge the solve at the edge holds the old contact. With 22 ohm in place of 220 ohm in `packages/engine/tests/circuits/button-led`, `#led1.i` is first over 1 ns after the press, and `over_ps` is exactly the time the contact was closed.

A solve that holds each capacitor at a voltage is not a sample. The operating point at t = 0 holds each `initial` voltage, and the restart solve at an edge holds each capacitor where the step left it. A capacitor held against a source draws what the hold takes there, 1e9 A per volt for 1 uF, and no part carries that. So the first accepted step after t = 0 is the first sample, and a sustained rating's filter holds 0 before it. After an edge, the next step's value holds back to the edge in the filter, since the step's own row holds the state before the edge.

A quantity is over when its magnitude, rounded to 3 significant digits, is above the rating rounded the same way. Equal is not over. `peak` is the largest magnitude. For an operating point it equals `abs(observations.op[ref])`. The operating point is still written, and `orbe-run run` exits 0.

### Sustained ratings

Each current and power rating has a basis, which its part takes from the datasheet, as [netlist.md](netlist.md) says. A peak rating is checked on each accepted step. A sustained rating, with a time constant `tau` and an average, RMS or mean, bounds the heat a held value makes. A transient checks it against a first-order filter of the samples, which starts at 0 as a cold part does: of their squares, read back as the root, for RMS, and of their magnitudes for mean. Each step's value holds until the next, as above. A value `x` held from cold reads `x * sqrt(1 - exp(-t / tau))` through an RMS filter and `x * (1 - exp(-t / tau))` through a mean one. So a load at `k` times the rating crosses after `tau * ln(k^2 / (k^2 - 1))` for RMS and `tau * ln(k / (k - 1))` for mean, and is [over now](#over-now) before that unless the rating has a peak limit. A sustained rating's `peak` is a second check on the same quantity: a sample over it warns at once as a peak rating, while the filter bounds the heat. A pulse much shorter than `tau` counts by its share of it. An operating point has no duration, so its value fills the filter: one over its rating warns at once, and the reading is its value. `t_ps` is the first step whose reading is over, `peak` the largest reading, `sample` the value at that step, and `over_ps` the time the reading was over.

A rating that names no basis takes its kind's. A power rating is a mean with a 10.2 ms time constant. A voltage source's `current` and an `atmega328p`'s `pin_current` and `supply_current` are RMS with 10.2 ms. Every other current rating is a peak.

An `atmega328p`'s `pin_current` and `supply_current` are DC ratings. DS40002061B §29.1 gives 40 mA and 200 mA as DC currents and gives no pulse rating, and a supply's rating is the current it delivers steadily. What a DC current rating guards against is heat, in the output transistors, the metal and the bond wires, and electromigration, which follows the mean current. Heat follows `i^2` over time, so these are RMS. RMS is never below the mean, so it covers electromigration too.

The time constant is 10.2 ms because PWM is the load a pin most often switches. Judging a PWM load by its RMS, as a DC rating's heat asks, assumes the pin heats slower than one period. The slowest PWM an Arduino core makes by default is Timer1's and Timer2's phase correct PWM on D3, D9, D10 and D11, 16 MHz / (2 * 64 * 255), 490.2 Hz, a period of exactly 2.04 ms, and 10.2 ms is five of those periods. A filter that slow reads a steady 490 Hz PWM within a few percent of its period's RMS, and a faster PWM closer still. `analogWrite(9, 128)` into 75 ohm is 50 mA at 128 / 255 duty, 35.4 mA RMS, under 40 mA, though each of its pulses lasts 1.02 ms. Into 47 ohm it is 69 mA at the same duty, 49 mA RMS, over it.

A slower PWM, such as a software PWM toggled with `delay`, counts each pulse longer than a few time constants as a held level: 100 mA for 50 ms reads 99.6 mA RMS. A shorter pulse counts by its share: 100 mA for 1 ms reads 31 mA RMS and does not warn, and the datasheet gives no pulse rating to hold it to. A capacitor that a pin charges through its stage, `R` ohm, takes `I0^2 * R * C / 2` of `i^2 * t` an edge. On the model's 25 ohm stage, 100 pF takes 0.2 A for a few nanoseconds, which reads well under 1 mA RMS, and a 1 uF load on a 490 Hz PWM pin stays well under 40 mA. 10 uF and 47 uF on that pin go over, as such a load does on a real pin, since every edge charges it again. A pin that charges 10 uF once from 0 V stays under; 47 uF and 100 uF go over. A held load crosses at the times above: 69 mA from a pin into 47 ohm after 4.1 ms, five LEDs taking 44 mA from one pin after 18 ms, and a pin driving low on 5V, 200 mA, after 0.42 ms. Until then it is over now.

The catalog's LEDs rate their DC forward current as a mean with a 1 ms time constant and their sheet's peak forward current as its peak, so a pulse train at that peak, 0.1 ms at 1/10 duty, stays quiet, a pulse over it warns, and a held current over the DC rating warns. The catalog's diodes rate their average forward current as a mean with a time constant that holds their sheet's surge under it, and that surge as the peak.

The 9 V LED example puts about 0.55 A through an LED rated 0.03 A.

A voltage source with `ratings.current` names its current, `#<part>.i`, over that rating. The USB sources of the catalog's boards carry 0.5 A, the most a high-power USB 2.0 function draws (Table 7-7, `ICCHPF`). A 2.5 ohm load from a Nano's 5V to its GND draws 1.8 A through the board's USB Schottky, so `#nano__usb__host.i` is over, and so is `#nano__diode.i`, the Schottky's own 0.5 A average forward current.

An `atmega328p` with `ratings.pin_current` names each port pin whose RMS reading is over it, such as `#u1.pin.PB1`. A pin's current is the current out of the pin into the circuit, through its output stage in the state the firmware drives, its pull-up and its leakage to both rails, as [netlist.md](netlist.md#atmega328p) stamps them. With `ratings.supply_current`, `#u1.pin.VCC` is the sum of every port pin's current drawn from `VCC`, and `#u1.pin.GND_8` the sum of every port pin's current into `GND_8`, which carries `GND_22` too. With `active_current`, the chip's own draw through its conductance from `VCC` to `GND_8` is in both sums, and while a reset holds the chip that is `reset_current`'s conductance when the part has one. A row is solved with the drive the firmware had in force until that step, so a pin that changes drive on a step is checked with its new drive from the next step on.

The number is the model's. The stage is linear, 25 ohm from its rail, so a pin shorted to the other rail reads 200 mA at 5 V. A real driver saturates, and a short draws about half that. Both are far over the 40 mA rating, so the warning holds. The rating is the datasheet's absolute maximum, which [avr.md](avr.md#ratings) names. A pin held over it can be damaged.

A Nano running a sketch that drives D9 high into 47 ohm to ground names `#nano__mcu.pin.PB1`, which carries 69 mA and crosses its 0.04 A rating 4.1 ms after the pin goes high.

The time over runs through the last accepted step, which a chip's core can leave behind the session's time.

Six pins each driving 120 ohm to ground carry 34.5 mA each, under the pin rating, and 207 mA together, so `#nano__mcu.pin.VCC` is over its 0.2 A.

### Over now

A sustained rating can be far over for a while before its filter crosses: a load at `k` times a power rating takes `tau * ln(k / (k - 1))`. So a session and a run folder also list each such rating that is over now, as a `rating.current` or `rating.power` whose `data.now` is `true`. A rating is over now when the value of the last sample is over the rating, the average over the last 2.04 ms is over it too, and the filter has not crossed. A rating with a peak limit is never over now. Its sheet bounds a pulse by that limit and by the filter, whose time constant is fitted to the sheet's pulse, so a pulse the sheet allows stays quiet throughout and a pulse past the limit warns on it. A diode or zener power rating is never over now: its current peak warns at once past the surge, and the power filter warns when the heat crosses. An LED's power stays over now. A held overload still warns when the filter crosses. That average is the RMS for an RMS rating and the mean for a mean one. While the run is younger than 2.04 ms it runs from the first sample, and at the first sample it is that sample. At t = 0 the operating point is no sample, so a session that has accepted nothing since reads the step after it, which it has already solved.

2.04 ms is one period of the 490 Hz PWM. A PWM load whose RMS over a period is under its rating stays quiet, though each pulse is over it, and so does a switching edge that spends a few hundred nanoseconds over a power rating. A load held over the rating crosses that average within 2.04 ms, at once while the run is younger than that.

`t_ps` is the first of the samples that have each been over, through the last. `peak` is the filter's reading, still under the rating, and `peak_ps` the last sample. `sample` is that sample's value. `over_ps` is the time from `t_ps` to `peak_ps`. `held_ps` is how long that sample would have to hold for the filter to pass the rating. A session lists the rating's sustained warning when its filter crosses, beside the over-now warning it listed first. A run folder lists each rating over now at its last accepted step, so a run that stops before a held overload crosses still names it: `led-9v` run for 1 us lists `#led1.p` and `#r1.p` over now.

A Nano with a button from 5V to 22 ohm to GND, pressed at 50 ms and read 1 ms later:

```text
#r1.p is 0.989 W now, over its 0.25 W rating; it warns if held about 2000000000 ps
```

The filter crosses 2.98 ms after the press. The step at 53 ms reads:

```text
#r1.p averages 0.252 W with a 10.2 ms time constant while it is 0.989 W, over its 0.25 W rating
```

## rating.power

Severity `warning`. Refs is the rated quantity, `#<part>.p`. `t_ps` is the first time the rating's reading is over it, 0 for the operating point.

The message takes the forms of `rating.current`'s, in W:

```text
{ref} averages {peak} W with a {tau} ms time constant while it is {sample} W, over its {limit} W rating for {over_ps} ps
{ref} is {sample} W now, over its {limit} W rating; it warns if held about {held_ps} ps
```

The second is a rating [over now](#over-now). Values are rounded, and ` while it is {sample} W` and ` for {over_ps} ps` are left out, as for `rating.current`. `data` is the same as `rating.current`'s.

A power rating is the heat the part can shed. Every kind that has one, a resistor, a diode, an LED, a zener, a bipolar transistor, a MOSFET and a linear regulator, is checked the same way: as a [sustained rating](#sustained-ratings), a mean with a 10.2 ms time constant unless its part names another basis. An operating point fills the filter with its value, so one over its rating warns at once. A switching edge that spends its rating for a few hundred nanoseconds stays under it. A load held at `k` times the rating crosses after `tau * ln(k / (k - 1))`, and is over now until then. A diode or zener power rating is never over now: its current peak warns at once past the surge, and the power filter warns when the heat crosses, as [over now](#over-now) says.

The 9 V LED example is a steady overload. It reports `#led1.p` over its 0.075 W rating and `#r1.p` over its 0.25 W rating.

## rating.voltage

Severity `warning`. Refs is the chip pin, `#<part>.pin.<pin>`. `t_ps` is the first time the pin is outside its window, 0 for the operating point.

The message is:

```text
{ref} reaches {peak} V over the chip's ground, outside its {low} V to {high} V rating for {over_ps} ps
```

A window with no floor reads `outside its {high} V ceiling`, and one with no ceiling `outside its {low} V floor`. Each voltage in the message is rounded to 3 significant digits. ` for {over_ps} ps` is left out as for `rating.current`. A voltage window is checked on each accepted step: a pin past a rail conducts into its clamp at once. A pin is outside its window when its voltage, rounded to 3 significant digits, sits below the rounded floor or above the rounded ceiling. Equal is inside.

`data` holds `high`, `low`, `over_ps`, `peak`, and `peak_ps`, unrounded. Each voltage is over the chip's ground pin, `GND_8` on an `atmega328p`. `peak` is the pin's voltage where it was farthest outside the window, and `peak_ps` is when. `low` and `high` are the window at that time, and either is `null` when the part rates none. `over_ps` is `null` for an operating point, which has no duration, and counts the time outside the window as `rating.current` counts the time over.

An `atmega328p` with a `ratings` object checks every pin but its two ground pins. The floor is `ratings.rail_margin` below ground. The ceiling is `ratings.rail_margin` above the chip's own `VCC`, so it follows the supply. `VCC`'s ceiling is `ratings.supply_voltage`, and `PC6`'s is `ratings.reset_voltage` while it is `RESET`. A pin exactly on the edge is inside. The model has no clamp diodes, so a pin outside the window draws no clamp current and its voltage is what the circuit drives. A real pin conducts into its clamp there, which is the damage the rating guards against.

A `bmp280` checks Table 3 the same way. `VDD` and `VDDIO` warn below -0.3 V or above 4.25 V over `GND`. `CSB`, `SDI`, `SCK` and `SDO` warn below -0.3 V or above `VDDIO` + 0.3 V. A pin exactly on the edge is inside. The part keeps answering past either edge. The model has no clamp diodes, so a pin outside the window draws no clamp current.

An `hc595` checks `SER`, `SRCLK`, `RCLK`, `SRCLR` and `OE` against `ratings.input_margin`, and `QA` to `QH` and `QHp` against `ratings.output_margin`. The floor is that margin below the chip's ground and the ceiling is that margin above its `VCC`. A pin exactly on the edge is inside, and the part keeps shifting. The model has no clamp diode, so a pin outside the window draws no clamp current. Section 6.1 of SCLS041J limits that current, IIK on an input and IOK on an output, to ±20 mA.

A `linear_regulator`'s `ratings.input_voltage` is the ceiling on `#<part>.pin.in` over its `gnd`, and an op-amp's `ratings.supply_voltage` the ceiling on `#<part>.pin.vcc`, as [netlist.md](netlist.md) says. An LM7805 on 40 V is outside its 35 V ceiling.

A Nano with a 9 V source through 1 kohm on A0 reads, at the operating point:

```text
#nano__mcu.pin.PC0 reaches 9 V over the chip's ground, outside its -0.5 V to 5.5 V rating
```

An `op_amp` uses the same code on `in_p` and `in_n` when `input_headroom` is positive. The ceiling is `V(vcc) - input_headroom` and there is no floor. The voltage is over the part's `gnd` pin, which the message calls the chip's ground. The differential stage does not change, so the output is still the comparison. An input exactly on the ceiling is inside. Zero rates no ceiling.

An LM358 comparator on a 5 V supply, `input_headroom` 1.5 V, with `in_p` at 4 V and `in_n` at 3.9 V, reads:

```text
#u1.pin.in_p reaches 4 V over the chip's ground, outside its 3.5 V ceiling
```

A resistor with `ratings.voltage` as `{ "min", "max" }` uses the same code on `#<part>.v`. The window is that operating voltage. The message names the peak of a drive that leaves a level under 0.2 V, not a pin over the chip's ground:

```text
#bz.v peaks at 0.692 V, outside its 3 V to 5 V operating voltage
```

A peak inside the window is quiet. A part that never leaves the low level, including DC, is quiet. The part still conducts. The time counts from the start of the first pulse outside the window to the end of the last, with each gap under 17 ms between them, and stops once the part rests longer than that. A pulse ends at its last sample at or above 0.2 V, so a long solver step into the rest after it adds nothing.

A resistor with a single-value `ratings.voltage` checks `#<part>.v` against `-limit` to `+limit` on every accepted step, and reads `#<part>.v reaches {peak} V, outside its {low} V to {high} V rating`.

A capacitor with `ratings.reverse_voltage` uses the same code on pin `a`. The floor is that rating below pin `b`, and there is no ceiling. The message calls pin `b` the chip's ground. A capacitor with no `ratings` object, or with `reverse_voltage` null, is not checked. A forward voltage is not checked. A voltage exactly on the floor is inside. A reverse rating of 0 warns on any reverse voltage, and 0 V is inside.

A `linear_regulator` uses the same code on pin `in`. The floor is -0.3 V over pin `gnd`, and there is no ceiling. The message calls `gnd` the chip's ground. A voltage exactly on -0.3 V is inside. The model has no diode from `out` to `in`, so a reversed input stays at the voltage the circuit drives and the warning is what names it.

An `analog_temp_sensor` checks `vs` over `gnd` using the same code. When `ratings.supply_max` is set, a supply above it (up to `ratings.supply_voltage`, when set) emits `#<part>.pin.vs reaches {peak} V over the chip's ground, above its {high} V maximum operating supply`. When `ratings.supply_voltage` is set, exceeding it emits `#<part>.pin.vs reaches {peak} V over the chip's ground, outside its {high} V ceiling`. A supply below `ratings.supply_min` is [`rating.supply_low`](#ratingsupply_low).
## rating.supply_low

Severity `warning`. Refs is the chip's supply pin, `#<part>.pin.<pin>`. `t_ps` is the first time the supply pin sat between the unpowered floor and its datasheet minimum, 0 for the operating point.

The message is:

```text
{ref} supply drops to {voltage} V over the chip's ground, below its {limit} V minimum for {over_ps} ps
```

When the minimum depends on the chip's clock frequency (such as an `atmega328p` per DS40002061B Figure 29-1), the message adds ` at {freq}` after `minimum`, for example `below its 3.78 V minimum at 16 MHz`. Each voltage in the message is rounded to 3 significant digits. ` for {over_ps} ps` is left out for an operating point, and while `over_ps` is 0. All warning episodes for one supply pin merge into one warning per run, naming the level the rail held continuously for at least 1 us most recently while above the unpowered floor and below the minimum (or the episode's upper bound when no warning episode lasted 1 us, or the operating-point voltage).

`data` holds `limit`, `over_ps`, `peak_ps`, and `voltage`, unrounded, plus `clock_hz` when the minimum depends on the clock frequency. `limit` is the part's declared `ratings.supply_min` (or `ratings.io_supply_min` on `bmp280`'s `VDDIO`, or the safe-operating-area minimum at `clock_hz` on `atmega328p`). `voltage` is `V(pin) - V(ground)` at `peak_ps`, the supply voltage the rail held continuously for at least 1 us most recently while strictly above the unpowered floor and below the minimum (for an operating point or when no warning episode lasted 1 us, the operating-point sample or the episode's upper bound), so nanosecond single-clock dips inside a longer low-supply episode do not replace the voltage the rail held. `over_ps` is `null` for an operating point, and sums the time spent below the minimum across warning episodes in the run.

A supply voltage at or below 0.1 V over the chip's ground is treated as unpowered (switched off or disconnected) and stays silent, and a supply at or above the minimum (once both voltages are rounded to 3 significant digits) stays silent. A clean passage through the band from off to in range or from in range to off is a normal power cycle and stays silent; a sag that leaves and returns above the minimum, a visit that enters from off and falls back to off without reaching the minimum, or a rail in the band when the run starts or ends warns `rating.supply_low` while the part keeps running as its model already behaves.

## rating.temperature

Severity `warning`. Refs is the part, `#<part>`. `t_ps` is when the world's temperature first left the part's operating range, 0 when it starts outside it.

The message is:

```text
{ref} sits at {peak} degC, outside its {low} degC to {high} degC operating temperature for {over_ps} ps
```

Each temperature in the message is rounded to 3 significant digits. ` for {over_ps} ps` is left out as for `rating.current`.

`data` holds `high`, `low`, `over_ps`, `peak` and `peak_ps`, unrounded. `low` and `high` are the part's `ratings.temperature`. `peak` is the level farthest outside the range, and `peak_ps` when the world reached it. `over_ps` is `null` for an operating point and sums the time outside through the last accepted step, every visit merged into one warning per part. `over_ratings` sums it through its probe, so a live session's `snapshot` counts the time up to `now`.

The world's temperature is a schedule of levels, so the check reads the schedule, not a row: the level at t = 0 holds from 0, each change holds from its time, and a change at or after the last accepted step is not reached. A level is outside when, rounded to 3 significant digits, it sits below the rounded `min` or above the rounded `max`. An end is inside. The part keeps running, and reads what its model reads there.

An operating range applies to a part that is operating, so only the time the part is powered counts. A part is powered while each of its supply pins, a `dht22`'s `VCC` and a `bmp280`'s `VDD` and `VDDIO`, sits above 0.1 V over its `GND`, the floor `rating.supply_low` treats as off. Each accepted step's supply holds until the next step, and the operating point's from t = 0. After a restart at an edge the next step's holds back to the edge, as a filter's sample does. A part that is never powered never warns, whatever the world's temperature.

A `dht22` and a `bmp280` carry the range, as [netlist.md](netlist.md#ratings) says. The catalog's DHT22 is -40 to 80 degC and its BMP280 -40 to 85 degC. Both minimums are the world's own floor, so only the maximum can be passed there. A DHT22 in a 110 degC world reads, after 2 s:

```text
#dht__sensor sits at 110 degC, outside its -40 degC to 80 degC operating temperature for 2000000000000 ps
```

## regulator.current_limit

Severity `warning`. Refs is the regulator's output pin, `#<part>.pin.out`. `t_ps` is the first time the regulator was held at its current limit, 0 for the operating point.

The message is:

```text
{pin} regulator output can give {limit} A at {target} V; the circuit asks for more, so it is giving {current} A and has dropped as low as {voltage} V for {over_ps} ps
```

Each value in the message is rounded to 3 significant digits. ` for {over_ps} ps` is left out for an operating point, and while `over_ps` is 0.

`data` holds `current`, `limit`, `over_ps`, `peak_ps`, `target`, and `voltage`, unrounded. `limit` is the part's `current_limit`. `target` is its `output_voltage`. `voltage` is `V(out) - V(gnd)` at `peak_ps`, the lowest output voltage that stayed at or below that level continuously for at least 1 us while the regulator was clamped once the warning triggered (for an operating point, the operating-point voltage), so picosecond-to-nanosecond switching spikes never count, and `current` is the pass current delivered there. `over_ps` is `null` for an operating point and sums the time spent clamped across the run, merged into one warning per regulator.

A regulator is clamped on a sample when its unregulated target `vo_min` is positive and the current-limit term `Rlim * d * sp((i - Ilim) / d)` subtracts at least 0.5% of `output_voltage` (about 99.7% of `current_limit` and above, so a load under the limit and a regulator merely in dropout stay quiet). An operating point warns at once. In a transient, a load that pulls an already-up rail down by at least 0.5% of `output_voltage` warns once the pull-down holds continuously for at least 1 us; shorter spikes, such as picosecond-to-nanosecond MOSFET gate-drain feedthrough on a turn-off edge under the limit, stay quiet because a real regulator's limit loop and output capacitor respond on microsecond timescales, while microsecond brownouts and PWM overloads still warn. While a clamped output starting below target keeps closing its remaining gap `drop = vo_min - vo` at a rate of at least `0.5 * drop` per second (checked over at least 500 us), such as an output capacitor charging at power-on alongside a load up to the limit, it stays quiet until it reaches regulation; because `drop` must decay at least exponentially from `vo_min` to `0.5% * output_voltage`, any real overload that does not reach regulation warns once it stalls below target, after 500 us with no capacitor, within a few `R * C` time constants with one, and in at most `ln(200) / 0.5` (~10.6 s) in any case.

## mosfet.body_diode

Severity `warning`. Refs is the MOSFET part, `#<part>`. `t_ps` is the first time the MOSFET's reverse bias exceeded its `body_diode_forward_voltage` in the triggering episode, 0 for the operating point.

The message is:

```text
{part} source sits {peak} V above drain, past its {forward_voltage} V body diode forward voltage; the body diode is not simulated, so a real part would conduct from source to drain for {over_ps} ps
{part} drain sits {peak} V above source, past its {forward_voltage} V body diode forward voltage; the body diode is not simulated, so a real part would conduct from drain to source for {over_ps} ps
```

The first is for an `nmos` and the second is for a `pmos`. Each voltage in the message is rounded to 3 significant digits. ` for {over_ps} ps` is left out for an operating point, and while `over_ps` is 0.

`data` holds `channel`, `forward_voltage`, `over_ps`, `peak`, and `peak_ps`, unrounded. `channel` is `"nmos"` or `"pmos"`. `forward_voltage` is the part's `body_diode_forward_voltage`. `peak` is the largest reverse bias across the omitted body diode (`V(source) - V(drain)` for `nmos`, `V(drain) - V(source)` for `pmos`) seen across warning episodes, and `peak_ps` is when it occurred. `over_ps` is `null` for an operating point and sums the time spent past `forward_voltage` across warning episodes in the run, merged into one warning per MOSFET.

The Level-1 `nmos` and `pmos` models omit the intrinsic body diode, so an off MOSFET reverse-biased past its datasheet diode knee shows no diode current where a real part conducts freely (such as in H-bridges, reverse-polarity wiring, or inductive kickback through a switch). A MOSFET without `body_diode_forward_voltage` is not checked. Reverse bias is past the threshold when, rounded to 3 significant digits, it is strictly greater than the rounded `body_diode_forward_voltage`; unpowered devices (0 V) and reverse voltages at or below the threshold stay quiet. An operating point warns at once. In a transient, reverse bias must hold continuously for at least 1 us before warning, so single solver steps and nanosecond switching edges (such as a bidirectional level shifter pulling low) stay quiet. The part keeps running as its model behaves; add an external `diode` from source to drain (for `nmos`) or drain to source (for `pmos`) if the circuit relies on the body diode path.
