# AGENTS.md

sim-engine is the electronics simulation engine: a resolved netlist in, a deterministic run folder out.

Notes and plans: ~/notes/orbefab/lines/sim-engine/ (repo Orbefab/notes)

Private notes live in `~/notes/orbefab` (Orbefab/notes). Follow the `notes` skill for what goes there vs here, and `notes-sync` to pull/push them.

## Crates

- `crates/core`, package `sim-core`, holds the netlist, the diagnostics, and the run folder.
- `crates/circuit`, package `sim-circuit`, solves the DC operating point.
- `crates/cli`, package `sim-cli`, is the `sim` binary.
- `crates/avr`, package `sim-avr`, models the ATmega328P core. `sim-circuit` runs it for each `atmega328p` part.
- `crates/xtensa`, package `sim-xtensa`, models the Xtensa core of the ESP32 and the ESP32-S3, with both generated opcode tables. `crates/xtensa/gen/regen.sh` regenerates the tables.
- `crates/esp32`, package `sim-esp32`, models the ESP32-S3 around `sim-xtensa`: both cores in one time order, SRAM and RTC memory, GPIO, its matrix and its interrupts, the interrupt matrix, SYSTIMER, the watchdogs, SPI2 and the GDMA channels that feed it, the SHA accelerator, LEDC's fixed duties, the peripheral clock gates and resets, the USB Serial/JTAG console, and for a chip booted from flash the ROM, SPI1 and the flash, the MMU, the eFuses and the rest of the boot path, from the captured state in `crates/esp32/src/s3/boot/state.txt`. `sim-circuit` runs it for each `esp32s3` part. `scripts/esp32s3-firmware.sh` rebuilds the bare-metal programs of `examples/esp32s3-blink` and `examples/esp32s3-pattern`, and with ESP-IDF v5.5.5 the apps and partition tables of `examples/esp32s3-hello`, `examples/esp32s3-screen` and `examples/esp32s3-encoder` from `firmware/esp32s3/idf`. `scripts/pattern_header.py` writes the pattern program's `pattern.h` from `pattern.json`. `scripts/blobs.sh` puts the ROM, which no example commits, into its cache folder and prints that folder.
- `crates/bench`, package `sim-bench`, times native, WASI and browser Worker runs. It is not published.
- `crates/wasm`, package `sim-wasm`, is the session and `sim run` for JavaScript. `scripts/wasm.sh` builds it, and `docs/wasm.md` is its guide.

## Before every commit

Run `scripts/check.sh` from the repo root. It runs:

```sh
cargo fmt --all --check
scripts/blobs.sh
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p sim-avr --all-targets --locked --features port-skip-check -- -D warnings
cargo test --workspace --locked --exclude sim-avr
cargo test -p sim-avr --locked --features port-skip-check
uv run scripts/ngspice_refs.py --self-test
uv run scripts/st77916_example.py --check
uv run scripts/render_st77916.py --self-test
uv run scripts/esp32s3_blink.py --self-test
uv run scripts/pattern_header.py --check
uv run scripts/esp32s3_hello.py --self-test
uv run scripts/esp32s3_screen.py --self-test
uv run scripts/esp32s3_encoder.py --self-test
```

It then runs `sim run` on `examples/st77916-replay` and checks the folder with `scripts/render_st77916.py --check`, on `examples/esp32s3-blink` and checks that folder with `scripts/esp32s3_blink.py --check`, on `examples/esp32s3-pattern` and checks that folder with `scripts/render_st77916.py --pattern firmware/esp32s3/pattern.json --check`, and on `examples/esp32s3-hello`, `examples/esp32s3-screen` and `examples/esp32s3-encoder` with the folder `scripts/blobs.sh` prints and checks those folders with `scripts/esp32s3_hello.py --check`, `scripts/esp32s3_screen.py --check` and `scripts/esp32s3_encoder.py --check`. `scripts/blobs.sh` fetches the ROM when its cache lacks it. It runs before the tests, since the dual-core probe test in `crates/esp32/tests/dualcore.rs` runs the ROM from that cache and fails when it is missing.

The commit check needs ngspice-42 on `PATH`.

The `sim-avr` lines turn on `port-skip-check` so the skip check runs, and the other crates leave the feature off.

Also run `scripts/wasm-check.sh` before a commit that touches `crates/wasm`, `crates/xtensa`, `crates/esp32`, `crates/circuit/src/session*`, `crates/circuit/src/active.rs`, `crates/circuit/src/active/`, `scripts/wasm.sh`, or the math in `crates/circuit/src/element/junction.rs`, `crates/circuit/src/step.rs` and `crates/core/src/netlist.rs` that goes through `libm`. `scripts/wasm-check.sh` needs bash 4.2 or later, node 22.7 or later, and rustup. `scripts/wasm.sh` needs bash 4.2 or later and rustup. The check compares the wasm package with native `sim run` in node Workers.
