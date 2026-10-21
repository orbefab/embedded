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

A gridless variant keeps the committed netlist and sets `grid_ps` to `stop_ps`. `max_step_ps` stays as committed. For blink, button, and analog-serial that cap equals the old grid, so those three rows do not measure the grid's cost. Fade's committed grid is 100 µs and its `max_step_ps` is 10 ms.

The serial rows are `atmega-analog-serial` stopped at 4 s, 8 s, and 16 s. Each job is timed whole, from t = 0. The grid and `max_step_ps` stay as committed. One native pass of every job in the set took 44 s, so the 16 s job stayed.

`simulate` minus the bare core is everything outside that sliced core run. That is the solves, the restarts, re-running the core after a short landing, and the core copies at commit.

## Columns

| Column | Meaning |
| --- | --- |
| example | Directory name under `examples/` |
| variant | `committed`, `gridless`, `serial-4s`, `serial-8s`, or `serial-16s` |
| simulated | `stop_ps` of that variant, in seconds |
| native, WASI, Worker | Median wall time of `simulate`, with the min and max across repeats |
| native x, WASI x, Worker x | Simulated seconds divided by that median |
| native core, WASI core, Worker core | That target's own bare-core median |
| landings | Accepted steps after t = 0 |
| stops | Chip stops on those steps, summed over chips |

A landing is an accepted step after t = 0. t = 0 is not a landing. Landings and stops stay out of the run folder. The runner exits nonzero when `landings`, `stops`, `end_ps`, or `simulated_ps` differ across native, WASI, and Worker, and a row missing on one target is the same error. A repeat whose counts differ from the first exits with an error.

The example names live in one Rust list. The runner reads them from the binary, and the Worker reads the file the runner wrote from that list. The target name comes from the build.

## The bar

`atmega-blink`, `atmega-button`, `atmega-fade`, and `atmega-analog-serial` need to run at 1.5 times real time or better in the browser Worker on the build machine. The ratio column is simulated time divided by wall time. The bar is a Worker ratio of 1.5 or more on the committed variant of each example.

## This run

The build laptop has 6 cores and 12 threads. The governor is `powersave`. Load before the quieter pass was 0.74 0.96 0.91. rustc 1.98.1, cargo 1.98.1, node v24.21.0, Chromium 156.0.8078.4, playwright 1.64.0. `perf` could not sample because `perf_event_paranoid` is 4. A separate one-repeat native run of the whole set, not the table below, was 99% user time, 0.32 s system, and 0 major faults.

Numbers are the quieter of two passes, 3 repeats each. The other pass started at load 0.90 and its native fade row sat near 0.89 s.

| example | variant | simulated | native | WASI | Worker | native x | WASI x | Worker x | native core | WASI core | Worker core | landings | stops |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| atmega-blink | committed | 4.000 | 0.820 (0.819 to 0.835) | 1.011 (0.993 to 1.043) | 1.032 (1.025 to 1.042) | 4.88 | 3.96 | 3.87 | 0.782 | 0.966 | 0.941 | 613 | 9 |
| atmega-blink | gridless | 4.000 | 0.829 (0.829 to 0.831) | 1.016 (1.012 to 1.018) | 1.040 (1.033 to 1.041) | 4.82 | 3.94 | 3.85 | 0.779 | 0.965 | 0.927 | 598 | 9 |
| atmega-button | committed | 1.000 | 0.518 (0.518 to 0.520) | 0.714 (0.713 to 0.715) | 0.721 (0.717 to 0.725) | 1.93 | 1.40 | 1.39 | 0.494 | 0.693 | 0.690 | 1082 | 34 |
| atmega-button | gridless | 1.000 | 0.514 (0.513 to 0.514) | 0.712 (0.712 to 0.713) | 0.716 (0.716 to 0.758) | 1.95 | 1.40 | 1.40 | 0.493 | 0.693 | 0.700 | 1075 | 34 |
| atmega-fade | committed | 1.570 | 0.720 (0.714 to 0.727) | 1.081 (1.081 to 1.093) | 1.145 (1.137 to 1.155) | 2.18 | 1.45 | 1.37 | 0.326 | 0.407 | 0.402 | 41938 | 1482 |
| atmega-fade | gridless | 1.570 | 0.614 (0.612 to 0.615) | 0.923 (0.918 to 0.930) | 0.980 (0.961 to 0.995) | 2.56 | 1.70 | 1.60 | 0.327 | 0.407 | 0.394 | 29668 | 1482 |
| atmega-analog-serial | committed | 0.019 | 0.020 (0.020 to 0.021) | 0.034 (0.034 to 0.035) | 0.036 (0.036 to 0.037) | 0.93 | 0.56 | 0.52 | 0.003 | 0.004 | 0.004 | 2514 | 100 |
| atmega-analog-serial | gridless | 0.019 | 0.019 (0.019 to 0.019) | 0.031 (0.031 to 0.031) | 0.033 (0.033 to 0.034) | 1.02 | 0.61 | 0.57 | 0.003 | 0.004 | 0.004 | 2296 | 100 |
| atmega-analog-serial | serial-4s | 4.000 | 4.379 (4.334 to 4.475) | 7.260 (7.092 to 7.399) | 7.446 (7.422 to 7.564) | 0.91 | 0.55 | 0.54 | 0.549 | 0.882 | 0.873 | 523534 | 20812 |
| atmega-analog-serial | serial-8s | 8.000 | 8.727 (8.638 to 9.003) | 14.610 (14.433 to 14.695) | 15.085 (14.975 to 15.112) | 0.92 | 0.55 | 0.53 | 1.105 | 1.690 | 1.722 | 1046919 | 41615 |
| atmega-analog-serial | serial-16s | 16.000 | 17.876 (17.633 to 19.145) | 29.069 (29.044 to 29.297) | 31.506 (29.484 to 32.218) | 0.90 | 0.55 | 0.51 | 2.209 | 3.338 | 3.546 | 2093844 | 83230 |

The serial jobs take the following wall seconds per simulated second, as a median and a range. Native 4 s, 8 s, and 16 s are 1.095 (1.083 to 1.119), 1.091 (1.080 to 1.125), and 1.117 (1.102 to 1.197). WASI is 1.815 (1.773 to 1.850), 1.826 (1.804 to 1.837), and 1.817 (1.815 to 1.831). The Worker is 1.862 (1.856 to 1.891), 1.886 (1.872 to 1.889), and 1.969 (1.843 to 2.014). The 4, 8 and 16 s rates lie in one spread on each target. Up to 16 s, any cost from the undrained transmit queue is below the noise.

Blink and button are the core. Bare core is 0.782 s of blink's 0.820 s, and 0.494 s of button's 0.518 s. In the Worker, button's bare core is 0.690 s of a 1 s run, so that ratio stays under 1.5 while the core takes that long. Blink's Worker ratio is 3.87.

Fade's core is 0.326 s of 0.720 s. The gridless run is 0.614 s. The committed Worker ratio is 1.37. The gridless Worker ratio is 1.60.

The committed serial example is 19 ms. Its core is 0.003 s of 0.020 s. The Worker ratio is 0.52. The 16 s row has the same shape. Its core is 2.209 s of 17.876 s.

An earlier table timed blink at 0.84 s native and 1.02 s under WASI. Blink's medians here are 0.820 s and 1.011 s. 0.84 s sits just above the native range, 0.819 s to 0.835 s. 1.02 s sits inside the WASI range, 0.993 s to 1.043 s.
