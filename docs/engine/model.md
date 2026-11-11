# Device models

`orbe-model`, in `packages/model`, is the contract a device implements. It is unstable: the crate is 0.x, and any release may change a trait or a type.

A built-in device is a folder in `packages/engine/src/device/<kind>/`. `model.rs` is the device, written against `orbe_model` and `std` alone. `mod.rs` holds the netlist struct, the JSON wire type, and the device's `Row`, which names the events it takes. `Model::device` in `packages/engine/src/netlist.rs` is the table: one arm per device kind maps a part to its row. `orbe-device-check` builds every `model.rs` in a crate whose only dependency is `orbe-model`, so a `model.rs` that names the engine, or any other crate, fails the workspace build. The CST816 is the first device on the contract. It is an I2C target. The WS2812 is a pulse target.

## State

A device is its state. `State` is `Copy + Send + 'static`: fixed-size plain data with no heap and no pointers. The engine keeps several copies of each part, runs one ahead, and keeps or drops it by copying, so a copy must be a whole checkpoint.

## Device

| Method | What it does |
| --- | --- |
| `new(params)` | Builds the device from its part's logic params, at power-on. Electrical params stay in the engine. |
| `run_until(cx, limit)` | Runs from the last time it reached. Returns the first stop in `(reached, limit]` where its drives may change or it samples, or `None` once it has done everything up to `limit`. A run split at any limit is the same run. |
| `drive()` | One `Drive` per output it declares, in order: high, low, pull-up or off. |
| `input(cx, at, levels)` | Its watches' levels from `at` on. |
| `sample(cx, at, volts)` | The volts of its samples at a stop that asked for them. |
| `event(cx, at, event)` | An event it declares, with fields the engine has checked against the declaration. |

`input`, `sample` and `event` never change the drives. What they cause comes at least 1 ps later, at a stop. The engine checks this and panics on a device that breaks it. `cx` is what a device may do during a call besides change its own state. `broke` names a rule and the time, and the first call wins. `view` replaces the bytes the host reads after the call, such as a chain's colours at a latch.

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

A `PulseTarget` is a device on a pulsed line. The engine watches the line, times each level, and calls the device when a level ends. The device sees the level and how long it held.

| Method | What it does |
| --- | --- |
| `pulse(cx, at, level, width)` | A level that held for `width` picoseconds ended at `at`. |
| `idle(cx, at, level)` | The line has held `level` for the idle time the host was given, at `at`. |

The WS2812 is the pulse target. It reads `pulse` and `idle`. [engine.md](engine.md#pulse-targets) gives what the engine keeps, and [engine.md](engine.md#records) gives the WS2812's windows.

## Events

A device's row declares each event it takes: its name, the netlist field that schedules it, and its fields, each an integer with a span or a bool. A part's netlist schedules events on that field, and a session applies one as `Setting::Event`. The engine checks the fields and delivers the event to `event` at its time. A row whose events or fields repeat a name does not compile.

## Not built yet

Nothing loads a device at run time. A built-in device is linked into the engine. The contract keeps loading possible later: state is plain data, a device file imports only `orbe_model` and `std`, and each call takes `cx` so a host can pass a context across a boundary. Not built: a macro that exports a device, a WebAssembly adapter and its host, commands that create, build and test a device, a browser loader, limits on a loaded device, and model items in the registry. A device that sends records or reads the world gets those calls when it lands.
