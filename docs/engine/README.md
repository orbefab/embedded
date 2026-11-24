# The engine

The engine reads a resolved netlist and writes a deterministic run folder. `orbe-scene` turns a scene into that netlist, and `orbe-session` runs it live.

The input is JSON. The folder holds `manifest.json`, `netlist.json`, `diagnostics.json`, and `observations.json`, and a transient run adds `traces/`. The engine solves resistors, voltage and current sources, diodes, LEDs, transistors, op-amps, capacitors, inductors, relays and timed switches at one operating point or over time, runs ATmega328P and ESP32-S3 firmware instruction by instruction, runs a 74HC595 shift register and an SG90 servo, and reports parts run past their ratings.

## Crates

| Folder | Crate | What it holds |
| --- | --- | --- |
| `packages/engine` | `orbe-engine` | The netlist, the diagnostics, the run folder, and the solver: the operating point, the transient, and the session. It runs a chip model for each chip part. Its `runner` feature adds `orbe-run`, the binary the checks run. |
| `packages/engine/device-check` | `orbe-device-check` | Builds each built-in device's `model.rs` with `orbe-model` and `std` alone, so a device that names anything else fails the workspace build. |
| `packages/model` | `orbe-model` | Unstable, 0.x. The contract a device implements: `Device`, `I2cTarget`, `PulseTarget`, `SpiTarget` and `DutyTarget`, the plain data they pass, `Time`, `Drive`, `Levels`, `Stop` and the records, and the state containers `Pages` and `Sha256`. [model.md](model.md) says what a device implements. |
| `packages/mcu` | `orbe-mcu` | Unstable, 0.x. The contract a microcontroller's chip crate implements: `Mcu`, the bytes and TWI phases it delivers, the records it sends, and its faults and notes. It depends on `orbe-model` alone. [mcu.md](mcu.md) says what a chip implements. |
| `packages/cores/xtensa` | `orbe-xtensa` | The Xtensa LX6 and LX7 core of the ESP32 and the ESP32-S3, with both generated opcode tables. `gen/regen.sh` regenerates the tables. |
| `packages/cores/avr` | `orbe-avr` | The AVR instruction set. A chip implements its bus. |
| `packages/socs/microchip` | `orbe-microchip` | The ATmega328P around `orbe-avr`, and its bridge on `orbe-mcu`. `orbe-engine` runs it for each `atmega328p` part. |
| `packages/socs/espressif` | `orbe-espressif` | The ESP32-S3 around `orbe-xtensa`: both cores in one time order, its memories, its peripherals and its boot path, from the captured state in `src/s3/boot/state.txt`, and its bridge on `orbe-mcu`. `orbe-engine` runs it for each `esp32s3` part. [esp32s3.md](esp32s3.md) says what it models. |

`packages/engine/tests/circuits` holds the netlists the tests and checks run, each with its blobs and references. `packages/engine/tests/firmware` holds their firmware sources and images. `packages/engine/scripts` holds the scripts that rebuild that firmware and check run folders apart from the engine.

## Cores and chips

A core crate in `packages/cores` models one instruction set: it fetches, decodes and executes, and reaches memory only through the bus its chip gives it. It never names a vendor's registers or memory map. A chip crate in `packages/socs` models one vendor's chip around a core: its memories, its peripherals and their registers, its clocks and its pins. It never decodes a CPU's instructions. A chip that needs a new instruction set adds a core crate, and a new chip on an existing core adds only a chip crate. A chip crate implements `orbe-mcu` and never depends on the engine. `scripts/check.sh fast` fails when a crate in `packages/cores` depends on a crate in `packages/socs`, when a crate in `packages/socs` depends on `packages/engine`, or when `orbe-mcu` depends on anything but `orbe-model`.

## Build and run

The engine's binary is `orbe-run`. It is internal: the tests, the checks and `scripts/run-hashes.sh` run it, and `orbe` does not ship it. It builds only with the `runner` feature of `orbe-engine`. A build of `orbe` or `orbe-wasm` leaves that feature off, so neither builds `orbe-run` or the report it prints.

```sh
cargo build -p orbe-engine --features runner --bin orbe-run
```

`packages/engine/tests/circuits/divider/netlist.json` is a 5 V source, a 1000 ohm resistor, and a 2000 ohm resistor. `#net.vout` is within 1e-8 relative of 10/3 V.

```sh
cargo run -q -p orbe-engine --features runner --bin orbe-run -- run packages/engine/tests/circuits/divider/netlist.json --out /tmp/orbe-run-divider
```

A clean run prints one JSON document and exits 0.

```json
{
  "status": "ok",
  "out": "/tmp/orbe-run-divider",
  "errors": 0,
  "warnings": 0,
  "diagnostics": []
}
```

`orbe-run run <netlist> --out <dir> [--blobs <dir>]…` looks up each blob the netlist names as [netlist.md](netlist.md#blobs) says.

The process exits with one of these codes.

- 0, `ok`. The folder is written. A warning still exits 0.
- 1, `failed`. The folder is written and contains an error diagnostic.
- 2, `usage_error`. The command line is wrong. Nothing is written.
- 3, `io_error`. The netlist cannot be read, or the folder cannot be written. For `view`, the run folder cannot be read, or an image cannot be written.

`--help` and `--version` print plain text and exit 0. Stdout for `run` repeats `diagnostics` in the same order as `diagnostics.json`.

## The run manifest

`scripts/run-hashes.sh` runs every netlist in `packages/engine/tests/circuits` and prints one `sha256  path` line per file those runs wrote. A `manifest.json` line is the hash of that JSON with `created_at` removed, the same removal `packages/engine/scripts/wasi-check.sh` uses when it compares two manifests. Every other file is hashed as written. The lines are sorted by path, so one circuit's files sit together in a diff.

Each Block adds three records. `blocks/<id>/netlist.json` is the netlist v1 its scene resolves to. `blocks/<id>/run/` is every file of that netlist run after `analysis.transient.stop_ps` is set to `2000000000000`, which is 2 s. The scene stops at 7200 s on a 1 ms grid, and `orbe-run` rejects that trace, so the manifest hashes this 2 s run instead. `blocks/<id>/orbe-test` is the stdout of `orbe test -v` on that Block. That stdout includes each `expect`'s measured value. A scene that names blobs, such as a DevKitC-1 app image and its partition table, has those bytes written into a directory, each file named by its SHA-256. The run is given that directory after the ROM cache.

A run that does not exit 0 stops the script. The script names the circuit and the exit code, prints the runner's report on stderr, and exits with that code. A Block test that fails prints its output on stderr and exits with the test's status. The paths are relative to the repo root, so two runs of one build print the same lines.

```sh
scripts/run-hashes.sh > /tmp/engine-before
# rebuild the engine
scripts/run-hashes.sh > /tmp/engine-after
diff -u /tmp/engine-before /tmp/engine-after
```

`--time` adds a `time circuits/<name> <milliseconds>` line for each circuit after the hashes. The number is the median wall time of five runs. Leave `--time` off when you diff two manifests.

The runs go through `orbe-run`. `RUNNER` selects the binary `scripts/run-hashes.sh` runs. When `RUNNER` is unset, the script builds release `orbe-run`.

## Check

[`scripts/check.sh`](../../scripts/check.sh) runs the checks in tiers, and its header says what each tier runs and needs. The header of [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) says what CI runs, and when. `fast` is lint and the Rust tests, except the engine tests marked `#[ignore = "full tier"]`. `full` adds the engine's slower checks: those tests, the chips' skip-check builds, the Python checkers under `packages/engine/scripts`, each example circuit checked by its script, the WASI and wasm package checks, the speed check, and `web`.

`scripts/core-deps.sh`, in `lint`, fails when a crate in `packages/cores` depends on a crate in `packages/socs`, when a crate in `packages/socs` depends on `packages/engine`, or when `orbe-mcu` depends on anything but `orbe-model`.

`port-skip-check` is the check that a skipped store changes no pin. `fast` runs clippy with that feature. `full` runs the `orbe-microchip` tests with it. `orbe-xtensa/poll-skip-check` is its ESP32-S3 counterpart, and `orbe-espressif/poll-skip-check` turns it on for the engine, which reaches the core only through the chip crate. On a step whose poll is skipped, that build reads the interrupt lines and panics if they moved. The read is the one a normal build makes when it does poll, so it settles and stamps on that path. It does not run the stamp a skipped poll uses.

`packages/engine/scripts/ngspice_refs.py --check` regenerates each committed ngspice reference in memory and fails when a byte differs, or when a skipped example already has `deck.cir` or `ngspice.json`. For an example with a chip or a relay it builds release `orbe-run` and runs the binary in cargo's target directory, so `CARGO_TARGET_DIR` selects it. [reference.md](reference.md) says what the reference holds. `packages/engine/scripts/wasi-check.sh` runs `orbe-run` built for `wasm32-wasip1` on every example in node and compares each folder with native `orbe-run`. [docs/wasm.md](../wasm.md#tests) describes `scripts/check-wasm.sh`, and [its speed section](../wasm.md#speed) `scripts/check-speed.sh`.

`packages/engine/scripts/blobs.sh` runs before the Rust tests in both tiers. It copies the ESP32-S3 ROM from `parts/esp32s3-qfn56` into its cache when the cache lacks it, and fetches Espressif's release only when that file is absent or does not hash to the pinned value. No engine example commits the ROM. The dual-core probe test in `packages/socs/espressif/tests/dualcore.rs` runs the ROM from that cache.

No tier builds firmware. `packages/engine/scripts/esp32s3-firmware.sh` rebuilds the bare-metal programs and, with ESP-IDF v5.5.5, the ESP-IDF apps. `packages/engine/scripts/esp32s3-arduino.sh --check console` rebuilds the console sketch, runs `packages/engine/tests/firmware/esp32s3/arduino/checks/console/netlist.json`, and checks the folder with `esp32s3_arduino_console.py --check`. `packages/engine/scripts/esp32s3-arduino.sh --check uart` rebuilds that sketch with CDC on boot disabled, so `Serial` is UART0, runs `packages/engine/tests/firmware/esp32s3/arduino/checks/uart/netlist.json`, and checks the folder with `esp32s3_arduino_uart.py --check`. `packages/engine/scripts/esp32s3-arduino.sh --check adc` rebuilds the adc sketch, runs `packages/engine/tests/firmware/esp32s3/arduino/checks/adc/netlist.json`, and checks the folder with `esp32s3_arduino_adc.py --check`. `packages/engine/scripts/esp32s3-arduino.sh --check knob LVGL_DIR` builds the knob sketch, runs `packages/engine/tests/firmware/esp32s3/arduino/checks/knob/netlist.json`, and checks the folder with `esp32s3_knob.py --check` against `refs.json`. Those refs come from a board session of three agreeing captures. Clockwise drives contact B.

## Pages

The schema is [netlist.md](netlist.md). How the engine steps, and how accurate it is, is [engine.md](engine.md). The folder is [run-folder.md](run-folder.md). The codes are [diagnostics.md](diagnostics.md). The ngspice references are [reference.md](reference.md). The ATmega328P behind the `atmega328p` kind is [avr.md](avr.md). The ESP32-S3 behind the `esp32s3` kind is [esp32s3.md](esp32s3.md). The contract a device implements is [model.md](model.md), and the one a microcontroller implements is [mcu.md](mcu.md). How to add an analog kind, and where a chip kind goes, is [add-a-kind.md](add-a-kind.md). The package that runs a Block in a browser Worker, and the check that every Block keeps up with real time there, is [docs/wasm.md](../wasm.md).

The compiled sketch images in `packages/engine/tests/firmware/sketch/` and in `blocks/*/build/` link third-party code under its own licenses. [NOTICE](../../packages/engine/tests/firmware/sketch/NOTICE) names the test images, and each Block's `firmware/NOTICE` names its image. The engine does not link that code. The catalog part `esp32s3-qfn56` carries the ESP32-S3 revision 0 mask ROM under Apache-2.0. `parts/esp32s3-qfn56/LICENSE-rom` is Espressif's licence, unchanged, and `parts/esp32s3-qfn56/NOTICE` quotes the third-party notices that ROM requires.
