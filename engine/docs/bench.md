# Benchmark

`scripts/bench.py` times the four committed chip examples. It builds `sim-bench` for native, WASI, and `wasm32-unknown-unknown`, runs each build, and writes one JSON report.

```sh
uv run --with playwright scripts/bench.py [report.json]
```

The default report is `.scratch/bench/report.json` next to the repo, and it is not committed. `--runs N` sets the repeat count and can sit anywhere on the command line. A missing value is an error. The default is 3.

The builds use `CARGO_BUILD_JOBS=4`. `CARGO_TARGET_DIR` is used when it is set. Otherwise the target directory is under `.scratch/bench` next to the repo, named `target-` plus a digest of this checkout's path, so another checkout does not share it. Intermediates go in a fresh directory under `.scratch/bench` and are removed when the run ends. The script stops when the filesystem that holds the target directory has under 20 GiB free. `rustup target add` runs for `wasm32-wasip1` and `wasm32-unknown-unknown` only when that target is missing.

## What a run times

Each workload is one parsed netlist. The repeat count times two regions.

- The full `simulate` call, after the netlist is parsed.
- For a netlist with one `atmega328p`, the bare core over the same simulated time. The core runs in slices of at most 1 ms. Pins and `Channel::Usart0` are drained after each slice. ADC channel 0 is held at 2.5 V. A nonzero `pin_overflow` fails the run.

Parsing, JSON editing, and writing the report are outside the clock. The clock is the caller's. Native and WASI use `Instant`. The Worker uses `performance.now` through one imported function. The page is served with `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`, so that clock is not coarsened. After the page loads, and before the first job, the runner reads `crossOriginIsolated` from the page and stores that value. It stops when the value is missing or false. `wasm-bindgen` is not a dependency.

A `live` row keeps the committed circuit, sets `grid_ps` to `stop_ps`, and sets `max_step_ps` to the smaller of 10 ms and `stop_ps`. Every example in this set stops at 19 ms or later, so the step is 10 ms. `atmega-blink`, `atmega-button`, and `atmega-fade` already use a 10 ms step, so a `live` row of those examples removes the output grid. `atmega-analog-serial`'s committed step is 20 µs, so its `live` row also lengthens the step. `live-4s`, `live-8s`, and `live-16s` stop that example at 4 s, 8 s, and 16 s and use the same grid and step. The committed row stays at 19 ms. Each job is timed whole, from t = 0.

A row is a probe, so a grid change does not move a step. On `atmega-blink`, `atmega-button`, and `atmega-fade` the step is already 10 ms, so the committed row and the `live` row take the same steps, and the time between them is the probes. `atmega-fade`'s committed grid is 100 µs, so that row writes a probe 100 times as often as the 10 ms grids.

`simulate` minus the bare core is everything outside that sliced core run. That is the solves, the restarts, re-running the core after a short landing, and the core copies at commit.

## Columns

| Column | Meaning |
| --- | --- |
| example | Directory name under `examples/` |
| variant | `committed`, `live`, `live-4s`, `live-8s`, or `live-16s` |
| simulated | `stop_ps` of that variant, in seconds |
| native, WASI, Worker | Median wall time of `simulate`, with the min and max across repeats |
| native x, WASI x, Worker x | Simulated seconds divided by that median |
| native core, WASI core, Worker core | That target's own bare-core median |
| landings | Accepted steps after t = 0 |
| stops | Chip stops on those steps, summed over chips |

A landing is an accepted step after t = 0. t = 0 is not a landing. Landings and stops stay out of the run folder. The runner exits nonzero when `landings`, `stops`, `end_ps`, or `simulated_ps` differ across native, WASI, and Worker, and a row missing on one target is the same error. A repeat whose counts differ from the first exits with an error.

The example names live in one Rust list. The runner reads them from the binary, and the Worker reads the file the runner wrote from that list. The target name comes from the build.

## The bar

`atmega-blink`, `atmega-button`, `atmega-fade`, and `atmega-analog-serial` need to run at 1.5 times real time or better in the browser Worker on the build machine. The ratio column is simulated time divided by wall time. The bar is a Worker ratio of 1.5 or more on each example's `live` row, and for `atmega-analog-serial` on `live-16s`. On 2026-10-21 the owner decided to measure the bar the way a live session runs each circuit, with no output grid and a 10 ms `max_step_ps`, while the committed netlists keep their fine settings for batch accuracy.

## This run

The build laptop has 6 cores and 12 threads, and the governor is `powersave`. Load before native was 9.46 9.14 31.08, before WASI was 4.74 7.65 27.78, before the Worker was 1.35 4.98 23.67, and after the last target was 1.09 3.36 20.09. rustc 1.98.1, cargo 1.98.1, node v24.21.0, Chromium 156.0.8078.4, playwright 1.64.0.

Numbers are 3 repeats of this build. Errors were 0. Landings and stops match across native, WASI, and the Worker. Every row's `end_ps` equals its stop.

| example | variant | simulated | native | WASI | Worker | native x | WASI x | Worker x | native core | WASI core | Worker core | landings | stops |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| atmega-blink | committed | 4.000 | 1.546 (1.479 to 1.555) | 1.029 (1.023 to 1.044) | 1.041 (1.038 to 1.080) | 2.59 | 3.89 | 3.84 | 1.456 | 0.976 | 0.943 | 598 | 9 |
| atmega-blink | live | 4.000 | 1.402 (1.374 to 1.537) | 1.021 (1.018 to 1.024) | 1.024 (1.018 to 1.024) | 2.85 | 3.92 | 3.91 | 1.371 | 0.966 | 0.910 | 598 | 9 |
| atmega-button | committed | 1.000 | 0.693 (0.685 to 0.696) | 0.470 (0.468 to 0.470) | 0.464 (0.463 to 0.464) | 1.44 | 2.13 | 2.16 | 0.598 | 0.418 | 0.427 | 1075 | 34 |
| atmega-button | live | 1.000 | 0.437 (0.426 to 0.525) | 0.467 (0.464 to 0.468) | 0.462 (0.461 to 0.463) | 2.29 | 2.14 | 2.16 | 0.408 | 0.417 | 0.427 | 1075 | 34 |
| atmega-fade | committed | 1.570 | 0.948 (0.844 to 1.008) | 1.139 (1.135 to 1.158) | 1.143 (1.142 to 1.153) | 1.66 | 1.38 | 1.37 | 0.386 | 0.416 | 0.388 | 29668 | 1482 |
| atmega-fade | live | 1.570 | 0.814 (0.733 to 0.904) | 0.940 (0.940 to 0.942) | 0.944 (0.943 to 0.945) | 1.93 | 1.67 | 1.66 | 0.417 | 0.409 | 0.389 | 29668 | 1482 |
| atmega-analog-serial | committed | 0.019 | 0.031 (0.031 to 0.040) | 0.042 (0.042 to 0.043) | 0.043 (0.042 to 0.044) | 0.62 | 0.45 | 0.45 | 0.003 | 0.004 | 0.004 | 2296 | 100 |
| atmega-analog-serial | live | 0.019 | 0.022 (0.019 to 0.025) | 0.026 (0.026 to 0.026) | 0.026 (0.026 to 0.026) | 0.84 | 0.73 | 0.74 | 0.004 | 0.004 | 0.004 | 1722 | 100 |
| atmega-analog-serial | live-4s | 4.000 | 3.551 (3.447 to 4.390) | 5.387 (5.369 to 5.395) | 5.383 (5.380 to 5.387) | 1.13 | 0.74 | 0.74 | 0.608 | 0.858 | 0.853 | 356326 | 20812 |
| atmega-analog-serial | live-8s | 8.000 | 6.850 (6.776 to 6.919) | 10.733 (10.645 to 10.751) | 10.815 (10.792 to 10.831) | 1.17 | 0.75 | 0.74 | 1.194 | 1.700 | 1.706 | 712494 | 41615 |
| atmega-analog-serial | live-16s | 16.000 | 16.558 (14.493 to 17.998) | 21.562 (21.449 to 21.570) | 21.774 (21.719 to 21.859) | 0.97 | 0.74 | 0.73 | 3.002 | 3.397 | 3.409 | 1424993 | 83230 |

The machine is shared, the load is recorded, and fine differences between rows are inside the noise.

`atmega-blink`'s `live` Worker ratio is 3.91, above 1.5. `atmega-button`'s `live` Worker ratio is 2.16, above 1.5. `atmega-fade`'s `live` Worker ratio is 1.66, above 1.5. `atmega-analog-serial`'s `live-16s` Worker ratio is 0.73, under 1.5.

`atmega-blink` accepts 598 landings and 9 stops on the committed row and the `live` row. `atmega-button` accepts 1075 landings and 34 stops on both rows.

`live-16s` accepts 1424993 landings and 83230 stops. That is 89062 landings per simulated second. A 10 ms step would accept 1600 landings over the same 16 s. The landings follow the chip's stops. A landing costs 11.6 µs on native, 15.1 µs on WASI, and 15.3 µs in the Worker.

On the Worker, `atmega-fade`'s committed run is 1.143 s and the `live` run is 0.944 s. Both rows accept 29668 landings and 1482 stops. The 0.199 s between the runs is the probes. The `live` core is 0.389 s, and the 0.555 s between the `live` run and that core is the stops. The committed repeats run from 1.142 s to 1.153 s, a spread of 0.011 s. The `live` repeats run from 0.943 s to 0.945 s, a spread of 0.002 s.

On WASI, `atmega-fade`'s committed run is 1.139 s and the `live` run is 0.940 s. The probe gap is 0.199 s. The committed repeats run from 1.135 s to 1.158 s, and the `live` repeats run from 0.940 s to 0.942 s.
