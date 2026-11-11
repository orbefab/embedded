# Device models

`orbe-model`, in `packages/model`, is the contract a device implements. It is unstable: the crate is 0.x, and any release may change a trait or a type.

A built-in device is a folder in `packages/engine/src/device/<kind>/`. `model.rs` is the device, written against `orbe_model` and `std` alone. `mod.rs` holds the netlist struct, the JSON wire type, and the device's `Row`, which names the events it takes. `Model::device` in `packages/engine/src/netlist.rs` is the table: one arm per device kind maps a part to its row. `orbe-device-check` builds every `model.rs` in a crate whose only dependency is `orbe-model`, so a `model.rs` that names the engine, or any other crate, fails the workspace build. Every built-in device is on the contract. The CST816 is an I2C target. The WS2812 is a pulse target. The ST77916 is an SPI target with a PWM pin, and shows a screen. The SPI and pulse replays drive their pins while the engine sends their records.

## State

A device is its state. `State` is `Copy + Send + 'static`: fixed-size plain data with no heap and no pointers. The engine keeps several copies of each part, runs one ahead, and keeps or drops it by copying, so a copy must be a whole checkpoint. A landing copies the state, so a device whose size depends on a param, such as an LED chain, takes its size as a const generic, and the engine builds the smallest that holds the part. The WS2812 comes in 16, 64, 256 and 1024 LEDs.

The engine copies a state only with `State::copy_from`, and only onto a copy made before or after it: it runs a copy ahead of the kept one and copies one over the other. The default copies the whole state. A large memory that calls write in parts goes in `Pages<PAGES, SIZE>`, fixed-size plain data like the rest of the state. Each page keeps the generation of its last write, and `Pages::copy_from` copies only the pages whose generations differ, so a device whose `copy_from` copies its pages that way pays for the pages a run wrote, not the whole memory. The ST77916's frame memory is 360 rows of 720 bytes in 45 pages of 8 rows, 259,200 bytes held in its state with no heap. A landing copies the pages its pixel writes touched.

Equal generations mean equal pages only between a state and a copy made before or after it. Two copies that both wrote since they split can share a generation with different bytes, so the engine never copies one over the other.

## Device

| Method | What it does |
| --- | --- |
| `new(params)` | Builds the device from its part's logic params, at power-on. Electrical params stay in the engine. |
| `run_until(cx, limit)` | Runs from the last time it reached. Returns the first stop in `(reached, limit]` where its drives may change or it samples, or `None` once it has done everything up to `limit`. A run split at any limit is the same run. |
| `drive()` | One `Drive` per output it declares, in order: high, low, pull-up or off. |
| `input(cx, at, levels)` | Its watches' levels from `at` on. |
| `sample(cx, at, volts)` | The volts of its samples at a stop that asked for them. |
| `event(cx, at, event)` | An event it declares, with fields the engine has checked against the declaration. |
| `view()` | What it shows now, the `View` that `SHOWS` declares. `View::Nothing` by default. |

`SHOWS` is what a device shows besides its drives, and `view()` returns the matching `View`.

- `Shows::Nothing`, the default.
- `Shows::Leds { kind, order }`, an LED chain. Its view is three bytes per LED in `order`.
- `Shows::Screen { kind, width, height, format }`, a screen. Its view is the frame memory, rows top to bottom, each pixel in `format`, and a `Panel`: whether the display is on, inverted, in reset or asleep, its MADCTL and COLMOD bytes, how many registers hold a write it stores and does not run, and the last duty its backlight pin took.

The engine lists a part in `observations.json` `leds` only when its device declares an LED chain, and in `displays` only when it declares a screen.

Only `run_until` changes the drives. `input`, `sample`, `event`, and every call of a role, such as `write` or `pulse`, leave them alone. What they cause comes at least 1 ps later, at a stop. The engine checks this after each of those calls, in every build, and panics on a device that breaks it.

`cx` is what a device may do during a call besides change its own state. The engine reads it after every call. `broke(rule, at)` names a rule and the time. The first time a device breaks each rule in a run is a `chip.protocol` warning, and the rule is the warning's `rule`, such as `bit`. One call keeps the first rule it names. A rule the engine does not know panics. `show()` says what an LED chain shows changed. The engine copies the view after that call, at the call's time, as a latch row. `drew(drawn)` is a pixel write a screen closed: its window, the bytes it took, and the first 8 bytes of their SHA-256, which `orbe_model::Sha256` makes. A call closes at most one. The engine writes it as a `display.csv` row at the call's time. A device that is not an LED chain and calls `show`, or is not a screen and calls `drew`, panics. A row from `run_until` takes the time of the stop it returns.

## I2C targets

An `I2cTarget` is a device on an I2C bus. The engine runs the bus: it watches `SCL` and `SDA`, matches the address, drives `SDA` for each ACK and read bit, and holds `SDA`'s timing. The device sees transfers and bytes.

| Method | What it does |
| --- | --- |
| `start(cx, at, dir)` | The master addressed it, to read or to write. |
| `write(cx, at, byte)` | A byte the master wrote. It returns whether to ACK. |
| `read(cx, at)` | The next byte the master reads. |
| `master_ack(cx, at, more)` | The master's ACK or NACK after a read byte. |
| `stop(cx, at)` | A STOP. |

[engine.md](engine.md#i2c-targets) gives the order of the calls on the bus.

## Pulse targets

A `PulseTarget` is a device on a pulsed line. The engine watches the line, times each level, and calls the device when a level ends. The device sees the level and how long it held. `IDLE` declares the level the line rests at and how many picoseconds a rest lasts before the line is idle.

| Method | What it does |
| --- | --- |
| `pulse(cx, at, level, width)` | A level that held for `width` picoseconds ended at `at`. |
| `idle(cx, at, level)` | The line has rested at `level`, `IDLE`'s level, for `IDLE`'s time, at `at`. |

The WS2812 is a pulse target whose line rests low. It reads `pulse` and `idle`, latches at `idle`, and shows its colours as an LED chain. [engine.md](engine.md#pulse-targets) gives what the engine keeps, and [engine.md](engine.md#records) gives the WS2812's windows.

## SPI targets

An `SpiTarget` is a device on an SPI bus. The engine takes each `Spi` record whose CS and SCLK sit on the device's pins. It drops one while the device's reset pin reads low, and one whose lanes sit on other nets than the device's data pins, which is `chip.link_miswired` at the first time. The device sees each other transaction whole, with its phases, data, lanes and whether CS is held at its end.

| Method | What it does |
| --- | --- |
| `transaction(cx, at, spi)` | A transaction that ended at `at`. |

The engine walks a landing's records on the concrete device, so a transaction's bytes and pixels never cross a call through a trait object.

## PWM pins

A `DutyTarget` has a PWM input pin. The engine gives it each `Duty` record on that pin's net. The ST77916's `BLK` is one, and its duty is the backlight in the screen's `Panel`.

| Method | What it does |
| --- | --- |
| `duty(cx, at, duty)` | The duty a chip set at `at`: `duty` of `2^resolution` steps at `hz`. |

A device on two buses takes a pair of engines. The ST77916 takes the SPI engine and the PWM pin.

## Replays

A replay is a part that plays recorded transactions onto its pins: `spi_replay` sends `Spi` records, and `pulses_replay` sends `Pulses` records. The records are the part's netlist data, so the engine's replay engine holds them, shared by every copy, and sends each at its end, at a stop with `record` set. The device is the pins. It drives them at their idle levels for the whole run: `CS` high and `SCLK` and `D0` to `D3` low, or `OUT` at the part's `idle`.

## Events

A device's row declares each event it takes: its name, the netlist field that schedules it, and its fields, each an integer with a span or a bool. A part's netlist schedules events on that field, and a session applies one as `Setting::Event`. The engine checks the fields and delivers the event to `event` at its time. A row whose events or fields repeat a name does not compile.

## Not built yet

Nothing loads a device at run time. A built-in device is linked into the engine. The contract keeps loading possible later: state is plain data, a device file imports only `orbe_model` and `std`, and each call takes `cx` so a host can pass a context across a boundary. Not built: a macro that exports a device, a WebAssembly adapter and its host, commands that create, build and test a device, a browser loader, limits on a loaded device, and model items in the registry. A device that sends records of its own, such as a pulse train it answers with, or reads the world gets those calls when it lands.
