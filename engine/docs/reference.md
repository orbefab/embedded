# ngspice references

Each example has an ngspice result next to its netlist. A DC example has an operating point, and a transient example has a waveform. A Rust test compares the engine with it. The Rust tests never run ngspice.

```text
examples/<name>/
  netlist.json
  deck.cir
  ngspice.json
```

## Regenerate

The script needs ngspice 42 on `PATH` and uv. On Debian and Ubuntu, `apt install ngspice` installs it.

```sh
uv run scripts/ngspice_refs.py                    # every examples/*/netlist.json
uv run scripts/ngspice_refs.py examples/led-9v    # one example
uv run scripts/ngspice_refs.py --check            # exit 1 on drift or an unchecked reference
uv run scripts/ngspice_refs.py --self-test        # skip contract, no ngspice
```

Stdout is one JSON document. It lists the examples written or checked, the examples skipped with a reason, and with `--check` the files that drifted. A skipped example that already has `ngspice.json` is a failure under `--check`. Its entry's `failure` is `committed ngspice.json was not checked`, and the process exits 1. A skip with no committed reference exits 0. `--self-test` checks those paths and the bounce generator's test vectors without ngspice. Its stdout is one JSON document.

Run it after any change to a `netlist.json`. The Rust test fails until you do.

## deck.cir

The script writes `deck.cir` from `netlist.json`. Do not edit it. `ngspice -b deck.cir` in that directory reruns it and writes `op.raw`, or `tran.raw` for a transient.

Parts are written in name order. Names are lowercased, and ground is node `0`. A resistor is `r<name>`, a voltage source is `v<name>`, a capacitor is `c<name>`, and a diode or LED is `d<name>` with its own `.model`. Numbers are written so they read back to the same f64.

The options are `temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12`. 25 C with `tnom` equal to `temp` matches the engine's 298.15 K with no temperature scaling. The tolerances are tighter than ngspice's defaults, so ngspice's own error stays far under the comparison bound.

## Transient decks

A switch is `s<name>`, a voltage-controlled switch with its own `sw` model, `vt=0.5 vh=0` and the part's `on_resistance` and `off_resistance`. A control source `vctl_<name>` drives it with a PWL waveform. The script expands the switch's events and bounce with the same integer generator as the engine, and each contact edge becomes a 1 ns ramp that starts at the edge's exact picosecond. ngspice has no ideal switch, so the ramp is the smallest change the comparison has to step around.

Each ramp corner also gets a source `vbp<k>` with a PWL corner one picosecond later. It makes that time a breakpoint, because ngspice skips a PWL corner it lands on an ulp early.

The analysis is `tran s p stop p 0 s p` with `s` one thousandth of `stop_ps`. ngspice picks its own points under that cap, which does not depend on the engine's `max_step_ps`. The options are the same as for an operating point.

A capacitor with an `initial` voltage adds `.ic v(<node>)=<initial>` for the pin that is not ground, negated when that pin is `b`. Without `uic`, ngspice holds the node there while it solves the operating point, then releases it. The node's voltage at t = 0 then matches the engine's. Its current does not. ngspice reports the capacitor's current at t = 0 as 0, and the engine reports the current the hold takes. So the transient comparison drops the t = 0 sample of that capacitor's `i` and `p` from both series before it reads them as piecewise linear. ngspice's 0 then does not enter the line the engine's first rows are compared against. A capacitor with `"initial": null` writes no line.

A switch and a held capacitor are translated only for a transient analysis. A held capacitor needs exactly one pin on ground. A transient deck reserves net and part names starting with `ctl_` or `bp`. Each of these skips the example.

## Skips

An example is skipped when a part has no `kind`, when it has a kind the script does not translate, when a part has no `pins` or is missing a pin, when two names are equal ignoring case, when a net other than ground is named `gnd`, which ngspice reads as ground, when a pin names an undeclared net, when a held capacitor is in an operating point deck or has no pin on ground, or when `netlist.json` has a shape the engine's parser would reject. The report lists the skip and the reason in the one JSON document.

## ngspice.json

```json
{
  "deck_sha256": "<sha256 of deck.cir>",
  "format": 0,
  "netlist_sha256": "<sha256 of netlist.json>",
  "ngspice": "ngspice-42",
  "op": { "#led1.i": 0.5501866029052672, "#net.vin": 9.0 },
  "options": "temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12"
}
```

A transient reference has `tran` in place of `op`.

```json
"tran": {
  "edge_ramp_ps": 1000,
  "switches": { "#sw1": [{ "state": "closed", "t_ps": 2000000000 }] },
  "t_s": [0.0, 8.1e-07],
  "values": {
    "#c1.i": [0.0, 0.0],
    "#c1.v": [0.0, 0.0],
    "#net.gnd": [0.0, 0.0],
    "#net.vout": [5e-05, 5e-05]
  }
}
```

`switches` is every contact edge before the stop, as the script expanded it. `t_s` is ngspice's time points in seconds. `values` holds every net voltage and each part's `i` and `v` at those points. The ground net is 0 on every row. `p` is not in the file.

`op` holds every net voltage and each part's `i` and `v`, under the engine's refs and sign rules. Values come from ngspice's ASCII raw file, which has 16 significant digits. `p` is left out. It is `v * i` on both sides.

## The comparison

`crates/cli/tests/ngspice.rs` runs each example. The run manifest's `input.sha256` must equal `netlist_sha256`. Then every key in `op` must satisfy

```text
|engine - ngspice| <= 1e-3 * |ngspice| + floor
```

The floor is 1e-9 for a voltage and 1e-12 for a current. It only matters near 0. The 1 pA floor does not absorb ngspice's junction gmin, `gmin` times `V`, in cutoff. A reverse-biased example needs a different comparison before it is added.

A transient example is checked in four steps.

1. The run has no diagnostics, and `observations.json`'s `switches` equals the reference's `switches`. Both equal `bounce::expand` on the parsed netlist.
2. The trace has a row at every edge and at the earlier of 1 ns after it and `stop_ps`.
3. Both waveforms are read as piecewise linear. The grid is the union of both time grids up to the engine's last row, without the open interval from each edge to that later row. The row at the edge holds the old contact. Every step after it is stamped with the new contact. ngspice ramps the switch across the same interval, so the interiors are not comparable.
4. On that grid, every trace column must satisfy

```text
max |engine - ngspice| <= 1e-3 * span + floor
```

The test derives each part's `p` as `v * i` at the samples of each side, then reads that series as piecewise linear. Each stored `p` must also equal that row's `v * i` within 1e-15 W. Every trace column is one of these comparisons.

`span` is the column's maximum minus its minimum in the reference. The floor is 1e-9 for a voltage, 1e-12 for a current, and 1e-15 for a power, so a constant column passes on its floor. The test prints each column's worst error and span when it fails.

Each engine step above the 1 ps floor holds every capacitor's local truncation error under 1e-6 of its voltage plus 1 nV, starting from a consistent restart after every edge. Error accumulates over steps, and the examples measure it. rc-switch is under 1e-6 of span from its closed form. A step at the floor is accepted over its tolerance, so a time constant under about 43 ps is not held to the 1e-3 bound above. [netlist.md](netlist.md#transient-analysis) gives the threshold and the measured error.

The tolerance is relative to each capacitor's voltage, not to how far that voltage moves, and the error a row carries is a few millionths of the level. So a swing smaller than about 1% of its level is not held to the 1e-3 bound, at the rows or between them. In one probe, 5 V feeds a rail through 0.47 ohm, 22 uF on the rail is held at 5 V, and a switch puts 220 ohm on the rail at 100 us. The rail sags 10.6 mV. At one row the engine is 1.02e-5 V from the closed form and 1.12e-5 V from ngspice, against a bound of 1.06e-5 V. That is 2e-6 of the 5 V level.

The bound reads each side as piecewise linear, so it also scores the chord between rows. Between two rows a step of `h` is a chord, which can be off the waveform by about `h^2/8 * |v''|`. Because the tolerance follows the level, a small swing on a large level also gets long steps. In another probe, 12 V feeds a rail through 2 ohm, 4.7 uF on the rail is held at 12 V, and a switch puts 470 ohm on the rail at 50 us. The rail sags 51 mV. 2.6 us after the edge, `#net.vrail` misses ngspice by 7.6e-5 V against a bound of 5.1e-5 V, inside a 1.25 us step. The engine's rows at both ends of that step are 5e-6 V and 1.2e-5 V from the closed form, and ngspice is within 3.3e-6 V of it. The closed form's own chord across the step is 8.4e-5 V off. Eight more columns miss the same way, the rail's currents and powers among them. A smaller `max_step_ps` makes the trace denser once it is under the step the error control picks. Above that, halving it does not move the trace.

The engine and ngspice agree to about 3e-7 relative on the LED examples. That gap is consistent with ngspice 42 using older values of `k` and `q` than the exact SI ones. The engine puts its 1e-12 S gmin from every node to ground, and ngspice puts it across each junction. On the divider that difference is about 4e-9 relative.
