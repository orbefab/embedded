# Microcontrollers

`orbe-mcu`, in `packages/mcu`, is the contract a microcontroller's chip crate implements so the engine can run it. It is unstable: the crate is 0.x, and any release may change the trait or a type. It depends on `orbe-model` alone, for `Time`, `Drive`, `Stop`, the records and the air.

A microcontroller is a chip crate in `packages/socs`, such as `orbe-microchip` for the ATmega328P and `orbe-espressif` for the ESP32-S3. The chip crate holds the chip and its `bridge` module, which implements `Mcu` beside the registers it reads. A chip crate never depends on `orbe-engine`. `scripts/check.sh fast` runs `scripts/core-deps.sh`, which fails when a crate in `packages/socs` depends on `packages/engine`, or when `orbe-mcu` depends on anything but `orbe-model`.

The engine keeps what the circuit sees, in `packages/engine/src/mcu/`. `mod.rs` holds the table of kinds, one row per chip crate's type. Each kind has its own file there with its netlist struct, its JSON wire type and its parse, and the samples it builds from its part's pins and electrical params. Each file describes its pads in one `Pads`: the supply pins, the output stage and the leakage they share, and each pad's net, pull-up, pull-down and thresholds. `Pads::build` makes one output and one watch per pad from it, the same way for every kind. The engine parses an ESP32-S3 executable and builds the chip crate's chip from it. `orbe-espressif`'s `App::parse` reads the app on an ESP32-S3's flash, and the engine names its blobs. The engine reads an ATmega328P's Intel HEX with `orbe-microchip`'s `parse_hex`. `orbe-espressif` maps the ROM by section, reading its ELF bytes with the `object` crate and keeping its own windows and refusals.

## Mcu

A chip is its state. `Mcu` needs `Clone`, and a copy is a checkpoint. The engine keeps several copies of each part, runs one ahead, and copies one over another with `clone_from`, only onto a copy of the same chip.

| Item | What it does |
| --- | --- |
| `USARTS` | The datasheet name of each USART it sends on. `serial.csv` names each byte's USART with it. |
| `TWI` | Whether it has a TWI master, whose phases go to `i2c.csv`. `false` by default. |
| `SENDS` | Whether it sends records, which go to `link.csv`. `false` by default. |
| `WORLD` | The world quantities it reads at a sample, each when the netlist declares it. None by default. The ATmega328P's temperature sensor reads `temperature`. |
| `run_until(limit)` | Runs from the last time it reached. Returns the first stop in `(reached, limit]` where its drives may change, it samples, or a record it sent ends, or `None` once it has done everything up to `limit`. A run split at any limit is the same run. |
| `drive()` | One `Drive` per output, in the order the engine built its outputs. |
| `pulled_down()` | Whether each output pulls down to the chip's own GND pin, in the order of `drive()`. Empty by default, which pulls none down. |
| `held()` | Whether a reset holds it, which changes its own supply current. `false` by default. |
| `input(at, levels)` | Its watches' levels from `at` on, in the order the engine built its watches. |
| `sample(at, volts, world)` | The volts of its samples at a stop that asked for them, and each `WORLD` quantity the netlist declares, as it stands just before `at`. A chip that never asks keeps the default, which does nothing. |
| `deliver(through, sent, saw)` | Hands over each USART byte whose stop bit ended at or before `through`, as a `Frame`, and each TWI phase that ended by then, as a `Phase`. |
| `take_sent(sent)` | Hands over each record it sent at the stop `run_until` last returned with `record` set. |
| `answered(at, miso)` | The bytes the targets answered on MISO to the one record of the stop at `at` that reads, `0xFF` where none answered. It comes before the chip runs past `at`. Does nothing by default. |
| `in_flight(ahead, at)` | The records it started by `at` and has not ended, where the run ends at `at`. `ahead` is its copy run on from there. Empty by default. |
| `radio()` | Whether it has a radio, which makes it a station on the air for its whole run. `false` by default. |
| `tuned()` | The channel its radio listens on now, or `None` while its RF is off. `None` by default. |
| `take_aired(aired)` | Hands over the frame it put on the air that ended at the stop `run_until` last returned with `record` set, if one did. A radio sends one frame at a time. Sends none by default. |
| `hear(at, frame, signal)` | Another station's frame on its channel, which ended at `at`, heard at `signal`. Returns whether it acknowledges it. What it causes comes at least 1 ps later, at a stop. `Ack::None` by default. |
| `acked(at, ack)` | Whether any station acknowledged the frame it put on the air that ended at `at`. It comes before the chip runs past `at`. Does nothing by default. |
| `fault()` | The fault that ended its run, and its time. `None` by default. |
| `notes(through)` | What it noted at or before `through`, each at its time. Empty by default. |
| `read(output)` | When its firmware first read that output's pin as a digital input since the pin's drive last turned off, as far as it has run. A read while the drive is not off does not count. Once the drive is no longer off, it is still the read of the stretch that just ended. `None` by default, so the engine checks no input of a chip that keeps it. [diagnostics.md](diagnostics.md#chipfloating_input) says what the engine does with it. |

A clocked chip runs to cycle `floor(limit_ps * f / 1e12)` and puts a stop at cycle `c` at `ceil(c * 1e12 / f)` ps, counting from its clock domain's last frequency change, as [engine.md](engine.md#active-chips) says. The chip converts between picoseconds and its own cycles.

Only `run_until` changes the drives, `pulled_down()` and `held()`. `input` and `sample` leave them alone, and what they cause comes at least 1 ps later, at a stop. The ATmega328P stops where `RESET` low or a brown-out starts holding it and where the hold ends, so the engine stamps its reset draw from that stop, as it does a new drive. The engine checks this after each of those calls, in every build, and panics on a chip that breaks it.

## Pull-downs

A pin's pull-down sits beside its drive, not in it. `Drive` is `orbe-model`'s, and a device's `Drives` packs it in two bits, so a chip's pull-down is its own slice instead. An output pulls down only while its drive is `PullUp` or `Off`. With `Off` it pulls to GND alone. With `PullUp` it pulls to both rails, each through its own resistor, as the ESP32-S3 does with `FUN_WPU` and `FUN_WPD` both set. The engine stamps the pull-down only on an output whose part names a pull-down resistance. An ESP32-S3 part that names none refuses a store that sets `FUN_WPD` on a pad with a net, as [esp32s3.md](esp32s3.md#what-it-models) says, and writes nothing. The ATmega328P has no pull-down and keeps the default. Its outputs name none, so its stamps, and its runs, are the same as before the pull-down.

The engine compares `pulled_down()` wherever it compares the drives: a change at a stop restamps, and an `input`, a `sample` or a power-on input that changes it panics. An output that pulls down is not floating for [`chip.floating_input`](diagnostics.md#chipfloating_input), and its net reaches ground through it.

## Records

A chip says which of its outputs a record went out on, as an index in its drives. A `Sent` is a record and those outputs: an SPI transaction with the outputs of its CS, SCLK and data pins, or a pulse train or a duty with its one output. The engine names each output's net, numbers the chip's records in the order it sent them, and holds them until a commit stages them, as [engine.md](engine.md#records) says.

## The air

A chip with a radio is a station. It puts frames on the air with `take_aired`, listens on the channel `tuned()` names, hears other stations' frames with `hear`, and learns with `acked` whether its own were acknowledged. The types are `orbe_model::air`'s, as [model.md](model.md#the-air) says. Every chip keeps the defaults, which have no radio. The engine owns the air, as [engine.md](engine.md#the-air) says, and calls them only on a station: a chip whose `radio()` is `true`, which an `esp32s3` built with its radio is, as [esp32s3.md](esp32s3.md#the-wi-fi-radio) says.

## Faults and notes

A `Fault` ends the run. A `Note` is a warning, and the run goes on. Each is a code and its fields. The engine turns each into its diagnostic, field for field, and names each PC by the symbol the part's firmware gives it. `Pc` names its CPU by index, 0 for the first. On the ESP32-S3, 1 is the APP CPU, and the diagnostic's `app` field is set for it.

| `Fault` | Diagnostic |
| --- | --- |
| `UnsupportedInstruction` | `chip.unsupported_instruction` |
| `UnmappedFetch` | `chip.unmapped_fetch` |
| `Halted` | `chip.halted` |
| `UnsupportedSetting` | `chip.unsupported_setting` |
| `Watchdog` | `chip.watchdog` |
| `EfuseProgram` | `chip.efuse_program` |
| `FlashCommand` | `chip.flash_command` |
| `UnsupportedClock` | `chip.unsupported_clock` |
| `FaultRestart` | `chip.fault_restart` |
| `PowerOnRestart` | `chip.power_on_restart` |
| `UnmappedLoad` | `chip.unmapped_load` |

| `Note` | Diagnostic |
| --- | --- |
| `UnmodeledRegister` | `chip.unmodeled_register` |
| `Restart` | `chip.restart` |

`Unsupported`, `Setting`, `FlashRefusal` and `ResetReason` are the fields that say why. [diagnostics.md](diagnostics.md) lists each code.

## Dispatch

The table's `Mcu` holds one chip of any kind, and each call is a `match` on the kind. No call on a chip goes through a `dyn`, and the chip runs its instructions inside `run_until`, in its own crate. The engine calls `run_until` once per stop from another crate, and the release build does not inline across crates on its own, so a chip marks its `run_until` `#[inline]`. Without it the ESP32-S3's `esp32s3-pattern` run is about 7% slower.

## A new microcontroller

A new chip on an existing core is a chip crate, or a module of one, that implements `Mcu`. In the engine it adds its netlist struct, its parse and the outputs, watches and samples it builds in `src/mcu/<kind>.rs`, and one row in the table in `src/mcu/mod.rs`. Its netlist kind is a row in the `chip` group of `define_kinds!`, as [add-a-kind.md](add-a-kind.md#a-chip-or-a-device) says.

## What stays unstable

Everything in `orbe-mcu` may change before a chip that its designers did not write runs on it. The contract has no constructor. Each kind's file builds its chip with the chip crate's own `new`. The engine parses an ESP32-S3 executable because it owns the firmware rules and the symbol a diagnostic names. An AVR image stays in `orbe-microchip`, and the ROM's sections stay in `orbe-espressif`. A new chip's crate parses its own image and depends on `object` itself. The contract gains a constructor only when a chip needs the engine to call one.
