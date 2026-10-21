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

A gridless variant keeps the committed netlist and sets `grid_ps` to `stop_ps`. `max_step_ps` stays as committed. A row is a probe, so both variants take the same steps, and the gap between them is the rows' probes. Fade's committed grid is 100 µs and its `max_step_ps` is 10 ms. The other three examples' `max_step_ps` equals their grid.

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

The build laptop has 6 cores and 12 threads. The governor is `powersave`. Load before the quieter pass was 1.14 2.40 2.69, and 1.35 after it. rustc 1.98.1, cargo 1.98.1, node v24.21.0, Chromium 156.0.8078.4, playwright 1.64.0. `perf` could not sample because `perf_event_paranoid` is 4.

Numbers are the quieter of two passes, 3 repeats each. The other pass started at load 1.90 and ended at 3.24, and its Worker button gridless row reached 1.361 s.

| example | variant | simulated | native | WASI | Worker | native x | WASI x | Worker x | native core | WASI core | Worker core | landings | stops |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| atmega-blink | committed | 4.000 | 0.864 (0.864 to 0.865) | 1.014 (1.009 to 1.029) | 1.064 (1.051 to 1.070) | 4.63 | 3.94 | 3.76 | 0.810 | 0.917 | 0.944 | 598 | 9 |
| atmega-blink | gridless | 4.000 | 0.860 (0.858 to 0.864) | 1.023 (1.000 to 1.034) | 1.043 (1.028 to 1.047) | 4.65 | 3.91 | 3.83 | 0.814 | 0.939 | 0.920 | 598 | 9 |
| atmega-button | committed | 1.000 | 0.538 (0.536 to 0.538) | 0.717 (0.710 to 0.728) | 0.721 (0.699 to 0.729) | 1.86 | 1.40 | 1.39 | 0.512 | 0.680 | 0.700 | 1075 | 34 |
| atmega-button | gridless | 1.000 | 0.548 (0.532 to 0.551) | 0.721 (0.700 to 0.722) | 0.716 (0.714 to 0.719) | 1.82 | 1.39 | 1.40 | 0.511 | 0.677 | 0.678 | 1075 | 34 |
| atmega-fade | committed | 1.570 | 0.758 (0.757 to 0.761) | 1.099 (1.097 to 1.120) | 1.203 (1.186 to 1.205) | 2.07 | 1.43 | 1.31 | 0.339 | 0.387 | 0.402 | 29668 | 1482 |
| atmega-fade | gridless | 1.570 | 0.637 (0.637 to 0.641) | 0.975 (0.959 to 1.045) | 0.972 (0.970 to 0.975) | 2.46 | 1.61 | 1.62 | 0.339 | 0.402 | 0.397 | 29668 | 1482 |
| atmega-analog-serial | committed | 0.019 | 0.026 (0.026 to 0.026) | 0.043 (0.043 to 0.044) | 0.044 (0.044 to 0.045) | 0.74 | 0.44 | 0.43 | 0.003 | 0.004 | 0.004 | 2296 | 100 |
| atmega-analog-serial | gridless | 0.019 | 0.019 (0.019 to 0.020) | 0.033 (0.033 to 0.033) | 0.033 (0.033 to 0.033) | 0.97 | 0.58 | 0.57 | 0.003 | 0.004 | 0.004 | 2296 | 100 |
| atmega-analog-serial | serial-4s | 4.000 | 5.421 (5.360 to 5.592) | 9.093 (9.010 to 9.131) | 9.148 (9.132 to 9.241) | 0.74 | 0.44 | 0.44 | 0.566 | 0.870 | 0.875 | 478002 | 20812 |
| atmega-analog-serial | serial-8s | 8.000 | 11.175 (10.746 to 11.191) | 18.079 (17.975 to 18.143) | 17.823 (17.714 to 18.203) | 0.72 | 0.44 | 0.45 | 1.134 | 1.733 | 1.721 | 955873 | 41615 |
| atmega-analog-serial | serial-16s | 16.000 | 22.517 (22.154 to 22.573) | 35.375 (35.362 to 36.684) | 35.688 (35.357 to 35.943) | 0.71 | 0.45 | 0.45 | 2.291 | 3.554 | 3.451 | 1911750 | 83230 |

The serial jobs take the following wall seconds per simulated second, as a median and a range. Native 4 s, 8 s, and 16 s are 1.355 (1.340 to 1.398), 1.397 (1.343 to 1.399), and 1.407 (1.385 to 1.411). WASI is 2.273 (2.253 to 2.283), 2.260 (2.247 to 2.268), and 2.211 (2.210 to 2.293). The Worker is 2.287 (2.283 to 2.310), 2.228 (2.214 to 2.275), and 2.231 (2.210 to 2.246). The 4, 8 and 16 s rates lie in one spread on each target. Up to 16 s, any cost from the undrained transmit queue is below the noise.

Blink and button are the core. Bare core is 0.810 s of blink's 0.864 s, and 0.512 s of button's 0.538 s. In the Worker, button's bare core is 0.700 s of a 1 s run, so that ratio stays under 1.5 while the core takes that long. Blink's Worker ratio is 3.76. On both, a few hundred probes do not show, and the committed and gridless rows agree within their ranges.

Fade's core is 0.339 s of 0.758 s. The gridless run is 0.637 s on the same 29668 landings. The 0.121 s between them is the probes, under 8 us for each of the 15701 rows. The committed Worker ratio is 1.31. The gridless Worker ratio is 1.62.

The committed serial example is 19 ms. Its core is 0.003 s of 0.026 s, and the gridless run is 0.019 s on the same 2296 landings. Its 951 rows cost about 7 us each, a quarter of the run. Its `max_step_ps` equals its 20 us grid, and the steps no longer land on the rows, so nearly every row is a probe. The Worker ratio is 0.43. The 16 s row has the same shape. Its core is 2.291 s of 22.517 s.

Engine 0.4.0's table had blink at 0.820 s native, fade at 0.720 s, and the committed serial example at 0.020 s. Run five times each, alternating on this machine, the 0.4.0 and 0.5.0 `sim run` of blink both took a median of 0.84 s, and fade took 0.81 s and 0.83 s. Blink's bare core here is 0.810 s against 0.782 s then, so the machine, not the engine, is slower on blink. The serial example's move, 0.020 s to 0.026 s and 17.876 s to 22.517 s at 16 s, is its rows' probes.
