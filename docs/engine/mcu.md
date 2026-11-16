# Microcontrollers

`orbe-mcu`, in `packages/mcu`, is the contract a microcontroller's chip crate implements so the engine can run it. It is unstable: the crate is 0.x, and any release may change the trait or a type. It depends on `orbe-model` alone, for `Time`, `Drive`, `Stop` and the records.

A microcontroller is a chip crate in `packages/socs`, such as `orbe-microchip` for the ATmega328P and `orbe-espressif` for the ESP32-S3. The chip crate holds the chip and its `bridge` module, which implements `Mcu` beside the registers it reads. A chip crate never depends on `orbe-engine`. `scripts/check.sh fast` runs `scripts/core-deps.sh`, which fails when a crate in `packages/socs` depends on `packages/engine`, or when `orbe-mcu` depends on anything but `orbe-model`.

The engine keeps what the circuit sees, in `packages/engine/src/mcu/`. `mod.rs` holds the table of kinds, one row per chip crate's type. Each kind has its own file there with its netlist struct, its JSON wire type and its parse, and the outputs, watches and samples it builds from its part's pins and electrical params. The engine also parses the firmware and builds the chip crate's chip from it.

## Mcu

A chip is its state. `Mcu` needs `Clone`, and a copy is a checkpoint. The engine keeps several copies of each part, runs one ahead, and copies one over another with `clone_from`, only onto a copy of the same chip.

| Item | What it does |
| --- | --- |
| `USARTS` | The datasheet name of each USART it sends on. `serial.csv` names each byte's USART with it. |
| `TWI` | Whether it has a TWI master, whose phases go to `i2c.csv`. `false` by default. |
| `SENDS` | Whether it sends records, which go to `link.csv`. `false` by default. |
| `run_until(limit)` | Runs from the last time it reached. Returns the first stop in `(reached, limit]` where its drives may change, it samples, or a record it sent ends, or `None` once it has done everything up to `limit`. A run split at any limit is the same run. |
| `drive()` | One `Drive` per output, in the order the engine built its outputs. |
| `input(at, levels)` | Its watches' levels from `at` on, in the order the engine built its watches. |
| `sample(at, volts)` | The volts of its samples at a stop that asked for them. A chip that never asks keeps the default, which does nothing. |
| `deliver(through, sent, saw)` | Hands over each USART byte whose stop bit ended at or before `through`, as a `Frame`, and each TWI phase that ended by then, as a `Phase`. |
| `take_sent(sent)` | Hands over each record it sent at the stop `run_until` last returned with `record` set. |
| `in_flight(ahead, at)` | The records it started by `at` and has not ended, where the run ends at `at`. `ahead` is its copy run on from there. Empty by default. |
| `fault()` | The fault that ended its run, and its time. `None` by default. |
| `notes(through)` | What it noted at or before `through`, each at its time. Empty by default. |

That is three constants and nine methods, five of them with defaults. A clocked chip runs to cycle `floor(limit_ps * f / 1e12)` and puts a stop at cycle `c` at `ceil(c * 1e12 / f)` ps, counting from its clock domain's last frequency change, as [engine.md](engine.md#active-chips) says. The chip converts between picoseconds and its own cycles.

Only `run_until` changes the drives. `input` and `sample` leave them alone, and what they cause comes at least 1 ps later, at a stop. The engine checks this after each of those calls, in every build, and panics on a chip that breaks it.

## Records

A chip says which of its outputs a record went out on, as an index in its drives. A `Sent` is a record and those outputs: an SPI transaction with the outputs of its CS, SCLK and data pins, or a pulse train or a duty with its one output. The engine names each output's net, numbers the chip's records in the order it sent them, and holds them until a commit stages them, as [engine.md](engine.md#records) says.

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

Everything in `orbe-mcu` may change before a chip that its designers did not write runs on it. The contract has no constructor: each kind's file builds its chip with the chip crate's own `new`, from firmware the engine parsed. Firmware parsing may move into the chip crates, and a constructor may then join the contract.
