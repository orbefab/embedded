# Benchmark

`packages/engine/scripts/bench.py` times the five committed chip examples: the four `atmega328p` examples and `st77916-replay`. It builds `orbe-engine-bench` for native, WASI, and `wasm32-unknown-unknown`, runs each build, and writes one JSON report.

```sh
uv run --with playwright packages/engine/scripts/bench.py [report.json]
```

The default report is `.scratch/bench/report.json` in the repo, and it is not committed. `--runs N` sets the repeat count and can sit anywhere on the command line. A missing value is an error. The default is 3.

The builds use `CARGO_BUILD_JOBS=4`. `CARGO_TARGET_DIR` is used when it is set. Otherwise the target directory is under `.scratch/bench` in the repo, named `target-` plus a digest of this checkout's path, so another checkout does not share it. Intermediates go in a fresh directory under `.scratch/bench` and are removed when the run ends. The script stops when the filesystem that holds the target directory has under 20 GiB free. `rustup target add` runs for `wasm32-wasip1` and `wasm32-unknown-unknown` only when that target is missing.

## What a run times

Each workload is one parsed netlist. The repeat count times two regions.

- The full `simulate` call, after the netlist is parsed.
- For a netlist with one `atmega328p`, the bare core over the same simulated time. `st77916-replay` has none, so its core columns read `none`. The core runs in slices of at most 1 ms. Pins and `Channel::Usart0` are drained after each slice. ADC channel 0 is held at 2.5 V. A nonzero `pin_overflow` fails the run.

Parsing, JSON editing, and writing the report are outside the clock. The clock is the caller's. Native and WASI use `Instant`. The Worker uses `performance.now` through one imported function. The page is served with `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`, so that clock is not coarsened. After the page loads, and before the first job, the runner reads `crossOriginIsolated` from the page and stores that value. It stops when the value is missing or false. The raw `orbe_engine_bench.wasm` build does not depend on `wasm-bindgen`. The runner reads `/proc/loadavg` before native, before WASI, before the Worker, and after the last target.

A `live` row keeps the committed circuit, sets `grid_ps` to `stop_ps`, and sets `max_step_ps` to the smaller of 10 ms and `stop_ps`. Every `atmega328p` example stops at 19 ms or later, so the step is 10 ms. `st77916-replay` stops at 2 ms, so its `live` step is 2 ms, and its committed step is 50 µs. `atmega-blink`, `atmega-button`, and `atmega-fade` already use a 10 ms step, so a `live` row of those examples removes the output grid. `atmega-analog-serial`'s committed step is 20 µs, so its `live` row also lengthens the step. `live-4s`, `live-8s`, and `live-16s` stop that example at 4 s, 8 s, and 16 s and use the same grid and step. The committed row stays at 19 ms. Each job is timed whole, from t = 0.

A row is a probe, so a grid change does not move a step. On `atmega-blink`, `atmega-button`, and `atmega-fade` the step is already 10 ms, so the committed row and the `live` row take the same steps, and the time between them is the probes. `atmega-fade`'s committed grid is 100 µs, so that row writes a probe 100 times as often as the 10 ms grids.

`simulate` minus the bare core is everything outside that sliced core run. That is the solves, the restarts, re-running the core after a short landing, and the core copies at commit.

## Columns

| Column | Meaning |
| --- | --- |
| example | Directory name under `packages/engine/tests/circuits/` |
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

`atmega-blink`, `atmega-button`, `atmega-fade`, and `atmega-analog-serial` need to run at 1.5 times real time or better in the browser Worker on the build machine. The ratio column is simulated time divided by wall time. The bar is a Worker ratio of 1.5 or more on each example's `live` row, and for `atmega-analog-serial` on `live-16s`. The Worker session column is not that bar. On 2026-10-21 the owner decided to measure the bar the way a live session runs each circuit, with no output grid and a 10 ms `max_step_ps`, while the committed netlists keep their fine settings for batch accuracy.

## This run

The build laptop has 6 cores and 12 threads, and the governor is `powersave`. Load before native was 1.78 1.31 1.80, before WASI was 1.38 1.27 1.76, before the Worker was 4.60 2.20 2.03, and after the last target was 4.09 2.61 2.19. The owner said the machine was otherwise idle. rustc 1.98.1, cargo 1.98.1, node v24.21.0, Chromium 156.0.8078.4, playwright 1.64.0.

Numbers are 3 repeats of this build, engine `0.6.0`. `st77916-replay` joined the list at engine `0.23.0`, after this run, so the table has no row for it. Errors were 0. Landings and stops match across native, WASI, and the Worker. Every row's `end_ps` equals its stop.

| example | variant | simulated | native | WASI | Worker | native x | WASI x | Worker x | native core | WASI core | Worker core | landings | stops |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| atmega-blink | committed | 4.000 | 0.891 (0.869 to 0.898) | 1.040 (1.032 to 1.052) | 1.157 (1.042 to 1.194) | 4.49 | 3.84 | 3.46 | 0.831 | 0.966 | 0.989 | 425 | 9 |
| atmega-blink | live | 4.000 | 0.860 (0.843 to 0.915) | 1.043 (1.028 to 1.058) | 1.048 (1.037 to 1.099) | 4.65 | 3.84 | 3.82 | 0.799 | 0.985 | 0.957 | 425 | 9 |
| atmega-button | committed | 1.000 | 0.342 (0.342 to 0.343) | 0.488 (0.487 to 0.494) | 0.459 (0.458 to 0.460) | 2.93 | 2.05 | 2.18 | 0.338 | 0.468 | 0.429 | 219 | 34 |
| atmega-button | live | 1.000 | 0.342 (0.342 to 0.343) | 0.463 (0.451 to 0.483) | 0.456 (0.455 to 0.470) | 2.92 | 2.16 | 2.19 | 0.338 | 0.436 | 0.434 | 219 | 34 |
| atmega-fade | committed | 1.570 | 0.512 (0.512 to 0.512) | 0.702 (0.696 to 0.708) | 0.723 (0.719 to 1.135) | 3.07 | 2.24 | 2.17 | 0.325 | 0.398 | 0.400 | 1511 | 1482 |
| atmega-fade | live | 1.570 | 0.408 (0.408 to 0.409) | 0.544 (0.537 to 0.553) | 0.569 (0.548 to 0.577) | 3.84 | 2.89 | 2.76 | 0.326 | 0.409 | 0.402 | 1511 | 1482 |
| atmega-analog-serial | committed | 0.019 | 0.015 (0.015 to 0.015) | 0.026 (0.025 to 0.026) | 0.025 (0.025 to 0.026) | 1.26 | 0.74 | 0.75 | 0.003 | 0.004 | 0.004 | 1033 | 100 |
| atmega-analog-serial | live | 0.019 | 0.005 (0.005 to 0.005) | 0.008 (0.008 to 0.008) | 0.008 (0.008 to 0.008) | 4.09 | 2.45 | 2.46 | 0.003 | 0.004 | 0.004 | 122 | 100 |
| atmega-analog-serial | live-4s | 4.000 | 0.961 (0.960 to 0.961) | 1.493 (1.492 to 1.837) | 1.898 (1.817 to 2.426) | 4.16 | 2.68 | 2.11 | 0.541 | 0.900 | 1.156 | 20834 | 20812 |
| atmega-analog-serial | live-8s | 8.000 | 1.985 (1.981 to 1.986) | 3.092 (3.021 to 3.652) | 4.043 (3.884 to 4.064) | 4.03 | 2.59 | 1.98 | 1.082 | 1.759 | 2.161 | 41637 | 41615 |
| atmega-analog-serial | live-16s | 16.000 | 4.244 (4.237 to 4.247) | 7.949 (7.656 to 8.036) | 6.749 (6.620 to 7.745) | 3.77 | 2.01 | 2.37 | 2.159 | 4.327 | 3.548 | 83252 | 83230 |

atmega-analog-serial live-4s takes 0.240 wall seconds per simulated second on native (0.240 to 0.240).
atmega-analog-serial live-8s takes 0.248 wall seconds per simulated second on native (0.248 to 0.248).
atmega-analog-serial live-16s takes 0.265 wall seconds per simulated second on native (0.265 to 0.265).
atmega-analog-serial live-4s takes 0.373 wall seconds per simulated second on WASI (0.373 to 0.459).
atmega-analog-serial live-8s takes 0.387 wall seconds per simulated second on WASI (0.378 to 0.456).
atmega-analog-serial live-16s takes 0.497 wall seconds per simulated second on WASI (0.479 to 0.502).
atmega-analog-serial live-4s takes 0.474 wall seconds per simulated second on Worker (0.454 to 0.607).
atmega-analog-serial live-8s takes 0.505 wall seconds per simulated second on Worker (0.486 to 0.508).
atmega-analog-serial live-16s takes 0.422 wall seconds per simulated second on Worker (0.414 to 0.484).

The machine is shared, the load is recorded, and fine differences between rows are inside the noise.

All four examples clear the bar. The `live` Worker ratios are 3.82 for `atmega-blink`, 2.19 for `atmega-button`, and 2.76 for `atmega-fade`. `atmega-analog-serial`'s `live-16s` Worker ratio is 2.37, and its slowest repeat, 7.745 s, is 2.07.

The landings are the same on every target and under any load. `live-16s` accepts 83252 landings and 83230 stops, so every stop but 22 is one landing. `atmega-blink` accepts 425 landings for 9 stops, `atmega-button` 219 for 34, and `atmega-fade` 1511 for 1482, on the committed row and the `live` row.

On native, `live-16s` takes 4.244 s and the bare core is 2.159 s of it. In the Worker it takes 6.749 s and the bare core is 3.548 s of it. The rest is about 25 µs a landing on native and 38 µs in the Worker.
