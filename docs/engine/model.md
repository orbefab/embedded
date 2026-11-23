# Device models

`orbe-model`, in `packages/model`, is the contract a device implements. It is unstable: the crate is 0.x, and any release may change a trait or a type.

A built-in device is a folder in `packages/engine/src/device/<kind>/`. `model.rs` is the device, written against `orbe_model` and `std` alone. `mod.rs` holds the netlist struct, the JSON wire type, and the device's `Row`, which names the events it takes. A device kind is also a row in the `chip` group of `define_kinds!` in `packages/engine/src/netlist/catalog.rs`, which reads its JSON and gives `orbe check` its pins, and a `Chip` variant in `packages/engine/src/netlist.rs`. The compiler then names each `match` on `Chip` that needs an arm. One is `Model::device`, the device table, whose arm maps a part to its row. [add-a-kind.md](add-a-kind.md#a-chip-or-a-device) covers the row. `orbe-device-check` builds every `model.rs` in a crate whose only dependency is `orbe-model`, so a `model.rs` that names the engine, or any other crate, fails the workspace build. Every built-in device is on the contract. The CST816 is an I2C target. The SG90 is a pulse target that shows a shaft angle. The WS2812 is a pulse target. The DHT22 is a pulse target on an open-drain line that reads the world and answers with a train. The ST77916 is an SPI target with a PWM pin, and shows a screen. The 74HC595 watches its logic pins and drives push-pull outputs. The SPI and pulse replays drive their pins while the engine sends their records.

## State

A device is its state. `State` is `Clone + Send + 'static`. A clone is a checkpoint: the engine keeps several copies of each part, runs one ahead, and keeps or drops it by copying. A move is not a checkpoint. A landing copies the state, so a device whose size depends on a param, such as an LED chain, takes its size as a const generic, and the engine builds the smallest that holds the part. The WS2812 comes in 16, 64, 256 and 1024 LEDs.

Building a run clones each device whole three times: once for the operating point, and once each for the committed copy and the copy ahead. After that the engine copies a state only with `State::copy_from`, and only onto a copy made before or after it: it runs the copy ahead of the committed one and copies one over the other. The default copies the whole state. A large memory that calls write in parts goes in `Pages<PAGES, SIZE>`, fixed-size plain data like the rest of the state. Each page keeps the generation of its last write, and `Pages::copy_from` copies only the pages whose generations differ, so a device whose `copy_from` copies its pages that way pays for the pages a run wrote, not the whole memory. The ST77916's frame memory is 360 rows of 720 bytes in 45 pages of 8 rows, 259,200 bytes. Those bytes live in a `Box`. Moving the device copies a pointer, and `copy_from` still copies only the pages a pixel write touched. A `Copy` frame of that size is allocated on the wasm stack, which is 1 MiB, and the constructors that move it overflow that stack.

Equal generations mean equal pages only between a state and a copy made before or after it. Two copies that both wrote since they split can share a generation with different bytes, so the engine never copies one over the other.

## Device

| Method | What it does |
| --- | --- |
| `new(params)` | Builds the device from its part's logic params, at power-on. Electrical params stay in the engine. |
| `run_until(cx, limit)` | Runs from the last time it reached. Returns the first stop in `(reached, limit]` where its drives may change or it samples, or `None` once it has done everything up to `limit`. A run split at any limit is the same run. |
| `drive()` | One `Drive` per output it declares, in order: high, low, pull-up or off. |
| `draw()` | The `Draw` it takes from its supply: `amps` while the supply is at or above `holds_above_v`, fading as `3 x^2 - 2 x^3` of it below, with `x` the supply over `holds_above_v`, to none at 0 V. `Draw::NONE` by default. |
| `input(cx, at, levels)` | Its watches' levels from `at` on. |
| `sample(cx, at, volts)` | The volts of its samples at a stop that asked for them. |
| `event(cx, at, event)` | An event it declares, with fields the engine has checked against the declaration. |
| `view()` | What it shows now, the `View` that `SHOWS` declares. `View::Nothing` by default. |

`SHOWS` is what a device shows besides its drives, and `view()` returns the matching `View`.

- `Shows::Nothing`, the default.
- `Shows::Leds { kind, order }`, an LED chain. Its view is three bytes per LED in `order`.
- `Shows::Screen { kind, width, height, format }`, a screen. Its view is the frame memory, rows top to bottom, each pixel in `format`, and a `Panel`: whether the display is on, inverted, in reset or asleep, its MADCTL and COLMOD bytes, how many registers hold a write it stores and does not run, and the last duty its backlight pin took.

The engine lists a part in `observations.json` `leds` only when its device declares an LED chain, and in `displays` only when it declares a screen.

Only `run_until` changes the drives. Any call may change the draw. The host stamps the draw it read at the last stop, and a change from another call takes effect at a stop the host adds 1 ps after it. `input`, `sample`, `event`, and every call of a role, such as `write` or `pulse`, leave them alone. What they cause comes at least 1 ps later, at a stop. The engine checks this after each of those calls, in every build, and panics on a device that breaks it.

`cx` is what a device may do during a call besides change its own state. The engine reads it after every call. `broke(rule, at)` names a rule and the time. The first time a device breaks each rule in a run is a `chip.protocol` warning, and the rule is the warning's `rule`, such as `bit`. One call keeps the first rule it names. A rule the engine does not know panics. `show()` says what an LED chain shows changed. The engine copies the view after that call, at the call's time, as a latch row. `drew(drawn)` is a pixel write a screen closed: its window, the bytes it took, and the first 8 bytes of their SHA-256, which `orbe_model::Sha256` makes. A call closes at most one. The engine writes it as a `display.csv` row at the call's time. A device that is not an LED chain and calls `show`, or is not a screen and calls `drew`, panics. A row from `run_until` takes the time of the stop it returns. `replayed()` says the call replays a level a `Pulses` record carried. The record lands at its end, so such a call's time is at or before the device's own, and a drive it changes would land in the past. `world(quantity)` and `pulses(start, train)` are in [The world](#the-world) and [Pulse targets](#pulse-targets).

## The world

`WORLD` lists the world quantities a device reads, `temperature`, `humidity` or `light`. It is empty by default. The device declares them in its code, so the host fills exactly the quantities the code reads, and a part file cannot drift from them. A netlist with a part whose device reads a quantity must declare that quantity in its `world`, or the part is `netlist.world` with rule `undeclared` on field `kind`. A scene adds each quantity its devices read to the ones it must declare.

`cx.world(quantity)` is that quantity's value at the call's time: the value after every change strictly before that time, the rule the trace and `World::before` use. A call at a change's own time reads the value before it. For `run_until` the time is `limit`. The engine lands every core whose device reads a quantity at each change of that quantity, so a run from a commit never crosses a change and reads one value for its whole span. A quantity the device did not declare in `WORLD` panics. A device that reads nothing gets an empty `Cx`, and its calls cost what they cost before the world existed.

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

`OPEN_DRAIN` declares that the device can pull its line low. Its part has one output, the drain, and the line rises on the circuit's pull-up. Such a device answers with a train: `cx.pulses(start, train)` queues `train` from `start`, later than the call. Each level of a `Pulses` with `idle` high is a span: a low pulls the line low through the drain for its picoseconds, and a high releases it. After the last level the line is released. The host's pulse engine plays the train: it stops the core at each edge, at its picosecond, and the analog solve gives the line's fall and its rise through the pull-up. The engine still watches the line and calls `pulse` for each level it sees, the device's own levels included, timed between the watch's crossings. The train is part of the core's state, so a copy rewound into it plays the rest from where its commit stood. A device queues at most one train at a time. A call that queues one while another is pending, or a device that is not `OPEN_DRAIN` and queues one, panics. An open-drain line takes no records.

| Method | What it does |
| --- | --- |
| `pulse(cx, at, level, width)` | A level that held for `width` picoseconds ended at `at`. |
| `idle(cx, at, level)` | The line has rested at `level`, `IDLE`'s level, for `IDLE`'s time, at `at`. |

The WS2812 is a pulse target whose line rests low. It reads `pulse` and `idle`, latches at `idle`, and shows its colours as an LED chain. Its draw follows its latched colours. With `DOUT`, it queues each high past its last LED's bits a delay later and stops at each `DOUT` edge. A `pulse` call that `replayed()` cannot drive `DOUT`, so it breaks rule `dout_record` instead. The DHT22 is an open-drain pulse target whose line rests high. A low of at least 0.8 ms is a start, and it queues its 40-bit reply 30 us after the line rises. [netlist.md](netlist.md#dht22) gives its timings. [engine.md](engine.md#pulse-targets) gives what the engine keeps, and [engine.md](engine.md#records) gives the WS2812's windows.

## SPI targets

An `SpiTarget` is a device on an SPI bus. The engine takes each `Spi` record whose CS and SCLK sit on the device's pins. It drops one while the device's reset pin reads low, and one whose lanes sit on other nets than the device's data pins, which is `chip.link_miswired` at the first time. The device sees each other transaction whole, with its phases, data, lanes and whether CS is held at its end.

A transaction's `miso` says what the host reads back on MISO, D1: nothing, one byte beside each data byte in full duplex, or a number of bytes in a read phase after the data. Before `transaction`, the engine asks `answer` for each byte the host reads, in order. A byte no device answers reads `0xFF`, and where two devices answer one byte, a bit reads low when either drives it low.

| Method | What it does |
| --- | --- |
| `transaction(cx, at, spi)` | A transaction that ended at `at`. |
| `answer(cx, at, phases, data, index)` | The byte it shifts out after `phases` and `index` bytes of the data stage, where `data` is what the host sent before that byte. `None` by default, which leaves MISO alone. |

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

Nothing loads a device at run time. A built-in device is linked into the engine. The contract keeps loading possible later: a device file imports only `orbe_model` and `std`, and each call takes `cx` so a host can pass a context across a boundary. Not built: a macro that exports a device, a WebAssembly adapter and its host, commands that create, build and test a device, a browser loader, limits on a loaded device, and model items in the registry.
