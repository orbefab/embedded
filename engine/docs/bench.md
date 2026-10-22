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

The build laptop has 6 cores and 12 threads, and the governor is `powersave`. Load before native was 2.70 4.25 4.46, before WASI was 1.44 3.40 4.14, before the Worker was 1.73 3.02 3.90, and after the last target was 1.33 2.36 3.52. rustc 1.98.1, cargo 1.98.1, node v24.21.0, Chromium 156.0.8078.4, playwright 1.64.0.

Numbers are 3 repeats of this build. Errors were 0. Landings and stops match across native, WASI, and the Worker. Every row's `end_ps` equals its stop.

| example | variant | simulated | native | WASI | Worker | native x | WASI x | Worker x | native core | WASI core | Worker core | landings | stops |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| atmega-blink | committed | 4.000 | 0.844 (0.838 to 0.854) | 1.011 (1.002 to 1.020) | 1.039 (1.025 to 1.047) | 4.74 | 3.96 | 3.85 | 0.800 | 0.920 | 0.920 | 598 | 9 |
| atmega-blink | live | 4.000 | 0.834 (0.832 to 0.834) | 1.003 (1.002 to 1.006) | 1.025 (1.025 to 1.054) | 4.80 | 3.99 | 3.90 | 0.792 | 0.918 | 0.926 | 598 | 9 |
| atmega-button | committed | 1.000 | 0.353 (0.353 to 0.353) | 0.473 (0.470 to 0.607) | 0.467 (0.466 to 0.468) | 2.84 | 2.11 | 2.14 | 0.326 | 0.431 | 0.428 | 1075 | 34 |
| atmega-button | live | 1.000 | 0.352 (0.352 to 0.352) | 0.517 (0.513 to 0.522) | 0.466 (0.466 to 0.468) | 2.84 | 1.94 | 2.15 | 0.326 | 0.469 | 0.432 | 1075 | 34 |
| atmega-fade | committed | 1.570 | 0.695 (0.695 to 0.703) | 1.288 (1.119 to 1.343) | 1.019 (1.007 to 1.032) | 2.26 | 1.22 | 1.54 | 0.323 | 0.428 | 0.392 | 29668 | 1482 |
| atmega-fade | live | 1.570 | 0.590 (0.588 to 0.590) | 0.877 (0.826 to 0.962) | 0.843 (0.843 to 0.845) | 2.66 | 1.79 | 1.86 | 0.323 | 0.431 | 0.392 | 29668 | 1482 |
| atmega-analog-serial | committed | 0.019 | 0.022 (0.022 to 0.022) | 0.071 (0.058 to 0.072) | 0.036 (0.035 to 0.037) | 0.85 | 0.27 | 0.54 | 0.003 | 0.009 | 0.004 | 2296 | 100 |
| atmega-analog-serial | live | 0.019 | 0.014 (0.014 to 0.014) | 0.034 (0.029 to 0.045) | 0.022 (0.022 to 0.022) | 1.38 | 0.57 | 0.86 | 0.003 | 0.008 | 0.004 | 1722 | 100 |
| atmega-analog-serial | live-4s | 4.000 | 2.937 (2.913 to 2.939) | 5.897 (5.594 to 6.786) | 4.577 (4.557 to 4.614) | 1.36 | 0.68 | 0.87 | 0.580 | 1.066 | 0.870 | 356326 | 20812 |
| atmega-analog-serial | live-8s | 8.000 | 5.809 (5.783 to 5.918) | 8.942 (8.936 to 10.923) | 9.238 (9.219 to 9.274) | 1.38 | 0.89 | 0.87 | 1.160 | 1.701 | 1.732 | 712494 | 41615 |
| atmega-analog-serial | live-16s | 16.000 | 11.911 (11.865 to 11.975) | 18.079 (17.989 to 18.132) | 18.766 (18.761 to 19.063) | 1.34 | 0.89 | 0.85 | 2.305 | 3.375 | 3.459 | 1424993 | 83230 |

The machine is shared, the load is recorded, and fine differences between rows are inside the noise.

`atmega-blink`'s `live` Worker ratio is 3.90, above 1.5. `atmega-button`'s `live` Worker ratio is 2.15, above 1.5. `atmega-fade`'s `live` Worker ratio is 1.86, above 1.5. `atmega-analog-serial`'s `live-16s` Worker ratio is 0.85, under 1.5.

`atmega-blink` accepts 598 landings and 9 stops on the committed row and the `live` row. `atmega-button` accepts 1075 landings and 34 stops on both rows.

`live-16s` accepts 1424993 landings and 83230 stops. That is 89062 landings per simulated second. A 10 ms step would accept 1600 landings over the same 16 s. The landings follow the chip's stops. A landing costs 8.4 µs on native, 12.7 µs on WASI, and 13.2 µs in the Worker.

On the Worker, `atmega-fade`'s committed run is 1.019 s and the `live` run is 0.843 s. Both rows accept 29668 landings and 1482 stops. The 0.176 s between the runs is the probes. The `live` core is 0.392 s, and the 0.451 s between the `live` run and that core is the stops. The committed repeats run from 1.007 s to 1.032 s, a spread of 0.025 s. The `live` repeats run from 0.843 s to 0.845 s, a spread of 0.002 s.

On WASI, `atmega-fade`'s committed repeats run from 1.119 s to 1.343 s, a spread of 0.224 s. The `live` repeats run from 0.826 s to 0.962 s, a spread of 0.136 s. The committed spread is too wide to read a probe gap.
