# The engine

The engine reads a resolved netlist and writes a deterministic run folder. `orbe-scene` turns a scene into that netlist, and `orbe-session` runs it live.

The input is JSON. The folder holds `manifest.json`, `netlist.json`, `diagnostics.json`, and `observations.json`, and a transient run adds `traces/`. The engine solves resistors, voltage and current sources, diodes, LEDs, transistors, capacitors, inductors, relays and timed switches at one operating point or over time, runs ATmega328P and ESP32-S3 firmware instruction by instruction, and reports parts run past their ratings.

## Crates

| Folder | Crate | What it holds |
| --- | --- | --- |
| `packages/engine/core` | `orbe-engine-core` | The netlist, the diagnostics, and the run folder. |
| `packages/engine/circuit` | `orbe-circuit` | The solver: the operating point, the transient, and the session. It runs a chip model for each chip part. |
| `packages/cores/xtensa` | `orbe-xtensa` | The Xtensa LX6 and LX7 core of the ESP32 and the ESP32-S3, with both generated opcode tables. `gen/regen.sh` regenerates the tables. |
| `packages/socs/avr` | `orbe-avr` | The ATmega328P. `orbe-circuit` runs it for each `atmega328p` part. |
| `packages/socs/espressif` | `orbe-espressif` | The ESP32-S3 around `orbe-xtensa`: both cores in one time order, its memories, its peripherals and its boot path, from the captured state in `src/s3/boot/state.txt`. `orbe-circuit` runs it for each `esp32s3` part. [esp32s3.md](esp32s3.md) says what it models. |
| `packages/engine/cli` | `orbe-engine-cli` | The `sim` binary. |

`packages/engine/tests/circuits` holds the netlists the tests and checks run, each with its blobs and references. `packages/engine/tests/firmware` holds their firmware sources and images. `packages/engine/scripts` holds the scripts that rebuild that firmware and check run folders apart from the engine.

## Cores and chips

A core crate in `packages/cores` models one instruction set: it fetches, decodes and executes, and reaches memory only through the bus its chip gives it. It never names a vendor's registers or memory map. A chip crate in `packages/socs` models one vendor's chip around a core: its memories, its peripherals and their registers, its clocks and its pins. It never decodes an instruction. A chip that needs a new instruction set adds a core crate, and a new chip on an existing core adds only a chip crate.

## Build and run

```sh
cargo build -p orbe-engine-cli
```

The binary is `sim`. `packages/engine/tests/circuits/divider/netlist.json` is a 5 V source, a 1000 ohm resistor, and a 2000 ohm resistor. `#net.vout` is within 1e-8 relative of 10/3 V.

```sh
cargo run -q -p orbe-engine-cli -- run packages/engine/tests/circuits/divider/netlist.json --out /tmp/sim-divider
```

A clean run prints one JSON document and exits 0.

```json
{
  "status": "ok",
  "out": "/tmp/sim-divider",
  "errors": 0,
  "warnings": 0,
  "diagnostics": []
}
```

`sim view <run> --out <dir>` writes `display-<part>.png` into `<dir>` for each display part of a finished run folder, what the panel's glass showed at the end. [run-folder.md](run-folder.md#what-the-glass-shows) says how it draws them.

The process exits with one of these codes.

- 0, `ok`. The folder is written. A warning still exits 0.
- 1, `failed`. The folder is written and contains an error diagnostic.
- 2, `usage_error`. The command line is wrong. Nothing is written.
- 3, `io_error`. The netlist cannot be read, or the folder cannot be written. For `view`, the run folder cannot be read, or an image cannot be written.

`--help` and `--version` print plain text and exit 0. Stdout for `run` repeats `diagnostics` in the same order as `diagnostics.json`.

## The run manifest

`scripts/run-hashes.sh` runs every netlist in `packages/engine/tests/circuits` and prints one `sha256  path` line per file those runs wrote. A `manifest.json` line is the hash of that JSON with `created_at` removed, the same removal `packages/engine/scripts/wasi-check.sh` uses when it compares two manifests. Every other file is hashed as written. Each Block adds `blocks/<id>/netlist.json`, the netlist v0 its scene resolves to, and `blocks/<id>/orbe-test`, the stdout of `orbe test -v` on that Block. That stdout includes each `expect`'s measured value. The script does not run a Block's scene. The scene stops at 7200 s on a 1 ms grid, and `orbe-run` rejects that trace. The paths are relative to the repo root, so two runs of one build print the same lines.

```sh
scripts/run-hashes.sh > /tmp/engine-before
# rebuild the engine
scripts/run-hashes.sh > /tmp/engine-after
diff -u /tmp/engine-before /tmp/engine-after
```

`--time` adds a `time circuits/<name> <milliseconds>` line for each circuit after the hashes. The number is the median wall time of five runs. Leave `--time` off when you diff two manifests.

The runs go through `orbe-run`. Build it with the `runner` feature of `orbe-circuit`. A build of `orbe` or `orbe-wasm` leaves the feature off, so neither binary builds `orbe-run`.

```sh
cargo build -p orbe-circuit --features runner --bin orbe-run
```

`orbe-run run <netlist> --out <dir> [--blobs <dir>]…` writes the same folder as `sim run`, and it uses the same exit codes. `RUNNER` selects the binary `scripts/run-hashes.sh` runs. When `RUNNER` is unset, the script builds release `orbe-run`.

## Check

`scripts/check.sh fast` runs shellcheck on every tracked shell script and ruff (`ruff check` and `ruff format --check`) on every tracked Python file, then rustfmt, clippy, and `cargo nextest` for the Rust tests other than the `orbe-avr` suite and five engine targets. Clippy also builds `orbe-run` with `--features runner`. shellcheck follows each script's source directive. ruff is the `packages/cad` dev dependency, run with `uv run --project packages/cad --frozen`, and its config is the repo's `ruff.toml`. `scripts/check.sh full` runs `fast`, then:

- `cargo nextest run --profile ci -p orbe-avr --locked --features port-skip-check`.
- `packages/engine/circuit/tests/transient.rs`, `packages/engine/circuit/tests/session.rs`, `packages/engine/cli/tests/run.rs`, `packages/engine/cli/tests/session.rs` and `packages/engine/cli/tests/ws2812.rs`. Each test in those files is `#[ignore = "full tier"]`, and `full` runs that set with `--run-ignored only`.
- the `orbe-xtensa`, `orbe-espressif` and `orbe-engine-cli` clippy and `cargo nextest` runs with `orbe-xtensa/poll-skip-check`. On a step whose poll is skipped, that build reads the interrupt lines and panics if they moved. The read is the one a normal build makes when it does poll, so it settles and stamps on that path. It does not run the stamp a skipped poll uses.
- `--self-test` of `ngspice_refs.py`, `render_st77916.py`, `esp32s3_blink.py`, `esp32s3_hello.py`, `esp32s3_screen.py`, `esp32s3_encoder.py`, `i2c_decode.py`, `esp32s3_touch.py`, `esp32s3_dualcore.py`, `esp32s3_echo.py`, `esp32s3_ring.py`, `esp32s3_arduino_console.py` and `esp32s3_knob.py`.
- `st77916_example.py --check` and `pattern_header.py --check`.
- `orbe-run run` on `st77916-replay`, `esp32s3-blink`, `esp32s3-pattern` and the seven ESP-IDF circuits, `esp32s3-hello`, `esp32s3-screen`, `esp32s3-encoder`, `esp32s3-touch`, `esp32s3-dualcore`, `esp32s3-echo` and `esp32s3-ring`, each checked by its script with `--check`.
- the knob refs: it copies `packages/engine/tests/firmware/esp32s3/arduino/checks/knob/netlist.json` under `TMPDIR`, runs `esp32s3_knob.py script` on the refs into that copy, and fails when `cmp` shows a difference.
- `packages/engine/scripts/wasi-check.sh`, which runs `orbe-run` built for `wasm32-wasip1` on every example in node and compares each folder with native `orbe-run`.
- `scripts/check-wasm.sh`, the browser package's check, which [docs/wasm.md](../wasm.md#tests) describes.
- `scripts/check-speed.sh` on the package and registry that check built, which fails when a Block plays too slowly in Chromium, as [docs/wasm.md](../wasm.md#speed) says.

`ws2812_decode.py --self-test` is not in that list. `packages/engine/cli/tests/ws2812.rs` runs it, and that file is one of the five `full tier` targets above.

`port-skip-check` is the check that a skipped store changes no pin. `fast` runs clippy with that feature. `full` runs the `orbe-avr` tests with it. `packages/engine/scripts/blobs.sh` runs before the Rust tests in both tiers. It fetches the ESP32-S3 ROM, which no circuit commits, into its cache when the cache lacks it. The dual-core probe test in `packages/socs/espressif/tests/dualcore.rs` runs the ROM from that cache. The `fast` tier needs uv, shellcheck and cargo-nextest. The version is `nextest-version` in `.config/nextest.toml`. It installs none of them. When shellcheck or cargo-nextest is missing it prints the install command and stops. `full` also needs ngspice-42 on `PATH`, node 24 or later, the repo's Node dependencies, a Chromium as [docs/wasm.md](../wasm.md#speed) says, rustup with the `wasm32-wasip1` target, and the `wasm-bindgen` CLI that [docs/wasm.md](../wasm.md#build) names.

Neither tier builds firmware. `packages/engine/scripts/esp32s3-firmware.sh` rebuilds the bare-metal programs and, with ESP-IDF v5.5.5, the ESP-IDF apps. `packages/engine/scripts/esp32s3-arduino.sh --check console` rebuilds the console sketch, runs `packages/engine/tests/firmware/esp32s3/arduino/checks/console/netlist.json`, and checks the folder with `esp32s3_arduino_console.py --check`. `packages/engine/scripts/esp32s3-arduino.sh --check knob LVGL_DIR` builds the knob sketch, runs `packages/engine/tests/firmware/esp32s3/arduino/checks/knob/netlist.json`, and checks the folder with `esp32s3_knob.py --check` against `refs.json`. Those refs come from a board session of three agreeing captures. Clockwise drives contact B.

## Pages

The schema is [netlist.md](netlist.md). How the engine steps, and how accurate it is, is [engine.md](engine.md). The folder is [run-folder.md](run-folder.md). The codes are [diagnostics.md](diagnostics.md). The ngspice references are [reference.md](reference.md). The ATmega328P behind the `atmega328p` kind is [avr.md](avr.md). The ESP32-S3 behind the `esp32s3` kind is [esp32s3.md](esp32s3.md). The package that runs a Block in a browser Worker, and the check that every Block keeps up with real time there, is [docs/wasm.md](../wasm.md).

The compiled sketch images in `packages/engine/tests/firmware/sketch/` and in `blocks/*/firmware/` link third-party code under its own licenses. [NOTICE](../../packages/engine/tests/firmware/sketch/NOTICE) names the test images, and each Block's `firmware/NOTICE` names its image. The engine does not link that code.
