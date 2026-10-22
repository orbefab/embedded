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

The build laptop has 6 cores and 12 threads, and the governor is `powersave`. Load before native was 4.40 3.06 2.88, before WASI was 2.65 2.82 2.80, before the Worker was 7.87 4.26 3.30, and after the last target was 8.84 6.00 4.09. Other work on the machine held about six threads through the Worker run. rustc 1.98.1, cargo 1.98.1, node v24.21.0, Chromium 156.0.8078.4, playwright 1.64.0.

Numbers are 3 repeats of this build. Errors were 0. Landings and stops match across native, WASI, and the Worker. Every row's `end_ps` equals its stop.

| example | variant | simulated | native | WASI | Worker | native x | WASI x | Worker x | native core | WASI core | Worker core | landings | stops |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| atmega-blink | committed | 4.000 | 0.848 (0.846 to 0.868) | 1.023 (1.016 to 1.027) | 1.941 (1.892 to 2.049) | 4.72 | 3.91 | 2.06 | 0.796 | 0.925 | 1.727 | 425 | 9 |
| atmega-blink | live | 4.000 | 0.944 (0.928 to 1.033) | 1.110 (1.055 to 1.181) | 1.698 (1.171 to 1.705) | 4.24 | 3.60 | 2.36 | 0.826 | 1.070 | 1.611 | 425 | 9 |
| atmega-button | committed | 1.000 | 0.379 (0.364 to 0.426) | 0.956 (0.846 to 0.960) | 1.011 (0.978 to 1.013) | 2.64 | 1.05 | 0.99 | 0.374 | 0.910 | 0.891 | 219 | 34 |
| atmega-button | live | 1.000 | 0.408 (0.376 to 0.458) | 0.869 (0.466 to 0.939) | 1.036 (0.854 to 1.079) | 2.45 | 1.15 | 0.96 | 0.370 | 0.642 | 1.002 | 219 | 34 |
| atmega-fade | committed | 1.570 | 0.559 (0.532 to 0.579) | 1.208 (0.831 to 1.222) | 1.340 (1.189 to 1.375) | 2.81 | 1.30 | 1.17 | 0.337 | 0.664 | 0.732 | 1511 | 1482 |
| atmega-fade | live | 1.570 | 0.431 (0.425 to 0.437) | 0.660 (0.606 to 0.957) | 0.626 (0.564 to 0.652) | 3.65 | 2.38 | 2.51 | 0.340 | 0.451 | 0.451 | 1511 | 1482 |
| atmega-analog-serial | committed | 0.019 | 0.017 (0.016 to 0.017) | 0.025 (0.025 to 0.026) | 0.029 (0.029 to 0.030) | 1.14 | 0.75 | 0.66 | 0.003 | 0.004 | 0.005 | 1033 | 100 |
| atmega-analog-serial | live | 0.019 | 0.005 (0.005 to 0.005) | 0.008 (0.008 to 0.008) | 0.009 (0.009 to 0.016) | 3.70 | 2.42 | 2.03 | 0.003 | 0.004 | 0.005 | 122 | 100 |
| atmega-analog-serial | live-4s | 4.000 | 1.140 (1.044 to 1.155) | 2.733 (1.847 to 2.924) | 3.010 (2.555 to 3.113) | 3.51 | 1.46 | 1.33 | 0.617 | 1.598 | 1.778 | 20834 | 20812 |
| atmega-analog-serial | live-8s | 8.000 | 2.204 (2.156 to 2.229) | 5.196 (3.428 to 5.810) | 6.209 (4.531 to 6.211) | 3.63 | 1.54 | 1.29 | 1.212 | 2.331 | 3.579 | 41637 | 41615 |
| atmega-analog-serial | live-16s | 16.000 | 4.334 (4.220 to 4.384) | 11.147 (9.453 to 12.009) | 12.713 (11.461 to 12.992) | 3.69 | 1.44 | 1.26 | 2.184 | 6.688 | 7.087 | 83252 | 83230 |

The machine is shared, the load is recorded, and fine differences between rows are inside the noise.

The Worker ratios of this run do not read the bar. The bare core does not change with the step policy, and the Worker's `live-16s` core took 7.087 s here against 3.459 s in the run before, at a load near 8. The same core took 2.184 s on native, where the earlier run took 2.305 s. A Worker reading of the bar needs a run at a quieter load.

The landings are the same on every target and under any load. `live-16s` accepts 83252 landings and 83230 stops, so every stop but 22 is one landing. Engine `0.5.0` accepted 1424993 landings there, because each pin edge started a segment at 1 ns and grew the step back. `atmega-blink` accepts 425 landings for 9 stops, `atmega-button` 219 for 34, and `atmega-fade` 1511 for 1482, on the committed row and the `live` row.

On native, `live-16s` takes 4.334 s, 3.69 times real time, and the bare core is 2.184 s of it. The other 2.150 s is about 26 µs a landing.
