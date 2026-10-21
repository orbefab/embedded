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

A gridless variant keeps the committed netlist and sets `grid_ps` to `stop_ps`. `max_step_ps` stays as committed. A row is a probe, so both variants take the same steps, and the gap between them is the rows' probes. `atmega-fade`'s committed grid is 100 µs and its `max_step_ps` is 10 ms. The other three examples' `max_step_ps` equals their grid.

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

The build laptop has 6 cores and 12 threads. The governor is `powersave`. The one-minute load was 0.68 before the timed section and 1.17 after it. The three load numbers were 0.68 0.69 2.50 before and 1.17 1.07 1.82 after. rustc 1.98.1, cargo 1.98.1, node v24.21.0, Chromium 156.0.8078.4, playwright 1.64.0. `perf_event_paranoid` is 4, so `perf` cannot sample. An unreported `sim run` of `atmega-button` took 0.38 s of user time and 0.38 s of wall time, with 0 s of system time, at 100% of one core, and 0 major faults.

Numbers are 3 repeats of this build. Errors were 0. Landings and stops match across native, WASI, and the Worker.

| example | variant | simulated | native | WASI | Worker | native x | WASI x | Worker x | native core | WASI core | Worker core | landings | stops |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| atmega-blink | committed | 4.000 | 0.861 (0.860 to 0.861) | 1.030 (1.030 to 1.039) | 1.055 (1.048 to 1.065) | 4.65 | 3.88 | 3.79 | 0.817 | 0.922 | 0.941 | 598 | 9 |
| atmega-blink | gridless | 4.000 | 0.862 (0.856 to 0.864) | 1.016 (1.012 to 1.018) | 1.038 (1.038 to 1.039) | 4.64 | 3.94 | 3.85 | 0.814 | 0.922 | 0.927 | 598 | 9 |
| atmega-button | committed | 1.000 | 0.356 (0.351 to 0.369) | 0.457 (0.456 to 0.492) | 0.479 (0.477 to 0.480) | 2.81 | 2.19 | 2.09 | 0.334 | 0.414 | 0.430 | 1075 | 34 |
| atmega-button | gridless | 1.000 | 0.357 (0.354 to 0.362) | 0.457 (0.457 to 0.457) | 0.470 (0.469 to 0.480) | 2.80 | 2.19 | 2.13 | 0.337 | 0.413 | 0.431 | 1075 | 34 |
| atmega-fade | committed | 1.570 | 0.764 (0.761 to 0.764) | 1.133 (1.114 to 1.190) | 1.176 (1.166 to 1.179) | 2.06 | 1.39 | 1.33 | 0.338 | 0.391 | 0.397 | 29668 | 1482 |
| atmega-fade | gridless | 1.570 | 0.640 (0.636 to 0.643) | 0.919 (0.919 to 0.919) | 0.962 (0.959 to 0.962) | 2.45 | 1.71 | 1.63 | 0.336 | 0.391 | 0.392 | 29668 | 1482 |
| atmega-analog-serial | committed | 0.019 | 0.026 (0.026 to 0.026) | 0.041 (0.041 to 0.043) | 0.044 (0.044 to 0.045) | 0.74 | 0.46 | 0.43 | 0.003 | 0.004 | 0.004 | 2296 | 100 |
| atmega-analog-serial | gridless | 0.019 | 0.019 (0.019 to 0.019) | 0.031 (0.031 to 0.031) | 0.033 (0.033 to 0.033) | 0.98 | 0.61 | 0.58 | 0.003 | 0.004 | 0.004 | 2296 | 100 |
| atmega-analog-serial | serial-4s | 4.000 | 5.441 (5.373 to 5.508) | 8.686 (8.682 to 8.707) | 9.115 (9.051 to 9.135) | 0.74 | 0.46 | 0.44 | 0.598 | 0.850 | 0.889 | 478002 | 20812 |
| atmega-analog-serial | serial-8s | 8.000 | 10.829 (10.826 to 10.887) | 17.367 (17.282 to 17.668) | 17.632 (17.616 to 18.036) | 0.74 | 0.46 | 0.45 | 1.196 | 1.701 | 1.734 | 955873 | 41615 |
| atmega-analog-serial | serial-16s | 16.000 | 21.885 (21.713 to 22.514) | 35.048 (34.910 to 35.081) | 35.648 (35.516 to 35.682) | 0.73 | 0.46 | 0.45 | 2.392 | 3.402 | 3.464 | 1911750 | 83230 |

The serial jobs take the following wall seconds per simulated second, as a median and a range. Native 4 s, 8 s, and 16 s are 1.360 (1.343 to 1.377), 1.354 (1.353 to 1.361), and 1.368 (1.357 to 1.407). WASI is 2.171 (2.171 to 2.177), 2.171 (2.160 to 2.209), and 2.190 (2.182 to 2.193). The Worker is 2.279 (2.263 to 2.284), 2.204 (2.202 to 2.255), and 2.228 (2.220 to 2.230). The 4, 8, and 16 s rates overlap on each target. Up to 16 s, any cost from the undrained transmit queue is below the noise.

`atmega-blink` and `atmega-button` are the core. Bare core is 0.817 s of `atmega-blink`'s 0.861 s, and 0.334 s of `atmega-button`'s 0.356 s. In the Worker, `atmega-button`'s bare core is 0.430 s of a 1 s run, ratio 2.09. `atmega-blink`'s Worker ratio is 3.79. On both, the committed and gridless rows agree within their ranges. Against the previous quiet table, native `atmega-button` moved from 0.538 s to 0.356 s and native `atmega-blink` stayed at 0.861 s against 0.864 s. The Worker moved from 0.721 s to 0.479 s on `atmega-button` and from 1.064 s to 1.055 s on `atmega-blink`.

`atmega-fade`'s core is 0.338 s of 0.764 s. The gridless run is 0.640 s on the same 29668 landings. The 0.124 s between them is the probes, 7.9 us for each of the 15701 rows. The 0.302 s between the gridless run and the core is the stops. The committed Worker ratio is 1.33. The gridless Worker ratio is 1.63.

The committed `atmega-analog-serial` run is 19 ms. Its core is 0.003 s of 0.026 s, and the gridless run is 0.019 s on the same 2296 landings. Its 951 rows cost about 7 us each, a quarter of the run. Its `max_step_ps` equals its 20 us grid, and the steps no longer land on the rows, so nearly every row is a probe. The Worker ratio is 0.43. The 16 s row has the same shape. Its core is 2.392 s of 21.885 s.

Engine 0.4.0's table had `atmega-blink` at 0.820 s native, `atmega-fade` at 0.720 s, and the committed serial example at 0.020 s. Run five times each, alternating on this machine, the 0.4.0 and 0.5.0 `sim run` of `atmega-blink` both took a median of 0.84 s, and `atmega-fade` took 0.81 s and 0.83 s. `atmega-blink`'s bare core here is 0.817 s against 0.782 s then. The machine is slower on `atmega-blink`. The serial example's move, 0.020 s to 0.026 s and 17.876 s to 21.885 s at 16 s, is its rows' probes.
