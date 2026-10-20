# ngspice references

Each example has an ngspice result next to its netlist, except `examples/atmega-blink` and any other example with an `atmega328p` part, which [Skips](#skips) covers. A DC example has an operating point, and a transient example has a waveform. A Rust test compares the engine with it. The Rust tests never run ngspice.

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

Stdout is one JSON document. It lists the examples written or checked, the examples skipped with a reason, and with `--check` the files that drifted. For an example with a chip, `--check` also builds and runs the engine, reads the run's `traces/logic.vcd` back with the standard library, summarises each signal, and compares that with `observations.logic`. Its `logic` list holds one entry per such example, and one that does not match exits 1. A skipped example that already has `deck.cir` or `ngspice.json` is a failure under `--check`. Its entry's `failure` names them, as in `committed ngspice.json was not checked`, and the process exits 1. A skip with neither file exits 0. `--self-test` checks those paths, each source spelling's deck line, the bounce generator's test vectors, and the VCD reader and its summary, without ngspice. Its stdout is one JSON document.

Run it after any change to a `netlist.json`. The Rust test fails until you do.

## deck.cir

The script writes `deck.cir` from `netlist.json`. Do not edit it. `ngspice -b deck.cir` in that directory reruns it and writes `op.raw`, or `tran.raw` for a transient, and `edges.raw` too for a chip.

Parts are written in name order. Names are lowercased, and ground is node `0`. A resistor is `r<name>`, a voltage source is `v<name>`, a current source is `i<name>`, a capacitor is `c<name>`, and a diode or LED is `d<name>` with its own `.model`. Numbers are written so they read back to the same f64.

A current source is `i<name> <n> <p> dc <current>`, its pins swapped. ngspice's current flows from the first node through the source to the second, and the engine's leaves at `p`. Both an operating point deck and a transient deck save `@i<name>[current]`, the current the source drives, and the script negates it into the engine's `#<name>.i`.

A source with a waveform writes `dc <initial>` in an operating point deck, the value the engine's operating point holds. A transient deck is described below.

The options are `temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12`. 25 C with `tnom` equal to `temp` matches the engine's 298.15 K with no temperature scaling. The tolerances are tighter than ngspice's defaults, so ngspice's own error stays far under the comparison bound.

## Transient decks

A switch is `s<name>`, a voltage-controlled switch with its own `sw` model, `vt=0.5 vh=0` and the part's `on_resistance` and `off_resistance`. A control source `vctl_<name>` drives it with a PWL waveform. The script expands the switch's events and bounce with the same integer generator as the engine, and each contact edge becomes a 1 ns ramp that starts at the edge's exact picosecond. ngspice has no ideal switch, so the ramp is the smallest change the comparison has to step around.

A source waveform is `dc <initial> pwl(...)`. Its PWL starts at `0p <initial>`, and each jump at `J` adds `Jp <old>` and `(J+1000)p <new>`, a 1 ns ramp like a contact's. The knots stop after the first one at or past `stop_ps`. The deck never writes PULSE: ngspice replaces a zero rise or fall with the analysis's time step, so a PULSE edge would ramp over `stop_ps / 1000`, not 1 ns.

Each ramp corner, and each waveform knot in (0, `stop_ps`], also gets a source `vbp<k>` with a PWL corner one picosecond later. It makes that time a breakpoint, because ngspice skips a PWL corner it lands on an ulp early.

The analysis is `tran s p stop p 0 s p` with `s` one thousandth of `stop_ps`. ngspice picks its own points under that cap, which does not depend on the engine's `max_step_ps` or `grid_ps`. The options are the same as for an operating point.

A capacitor with an `initial` voltage adds `.ic v(<node>)=<initial>` for the pin that is not ground, negated when that pin is `b`. Without `uic`, ngspice holds the node there while it solves the operating point, then releases it. The node's voltage at t = 0 then matches the engine's. Its current does not. ngspice reports the capacitor's current at t = 0 as 0, and the engine reports the current the hold takes. So the transient comparison drops the t = 0 sample of that capacitor's `i` and `p` from both series before it reads them as piecewise linear. ngspice's 0 then does not enter the line the engine's first rows are compared against. Those columns are scored from the reference's next sample on, since nothing before it says what ngspice's current was. On a linearized grid that is `grid_ps`. A capacitor with `"initial": null` writes no line.

A switch, a held capacitor and a chip are translated only for a transient analysis. A held capacitor needs exactly one pin on ground. A transient deck reserves net and part names starting with `ctl_`, `bp` or `x_`. Each of these skips the example.

## Chip decks

An NE555 is written as the engine's model, element by element, with names under `x_<name>`:

```text
rx_u1_l1 vcc cont 5000.0
rx_u1_l2 cont x_u1_lo 5000.0
rx_u1_l3 x_u1_lo 0 5000.0
rx_u1_sup vcc 0 1875.0
* u1 latch: RESET overrides TRIG, TRIG overrides THRES (SLFS022K Table 6-1).
bx_u1_s x_u1_s 0 v = (V(vcc,0) > 0.7) ? ((V(thr,x_u1_lo) > 0) ? ((V(thr,cont) > 0) ? 0 : (V(x_u1_q) > 0.5 ? 1 : 0)) : 1) : 0
rx_u1_q x_u1_s x_u1_q 1
cx_u1_q x_u1_q 0 1e-12
bx_u1_oh vcc out i = (V(x_u1_q) > 0.5) ? (V(vcc,out) - 0.9) / 8.0 : 0
bx_u1_ol out 0 i = (V(x_u1_q) > 0.5) ? 0 : (V(out,0) - 0.016666666666666666) / 16.666666666666668
bx_u1_d dis 0 i = V(dis,0) / ((V(x_u1_q) > 0.5) ? 250000000.0 : 18.75)
```

- The ladder and the supply resistance are resistors. The lower ladder node is `x_<name>_lo`.
- The latch is a B-source whose next state is SLFS022K Table 6-1 over the three comparisons. Each comparison reads high strictly above its threshold, as the engine's watches do. The state is node `x_<name>_q`, held on 1 ohm and 1 pF, a 1 ps time constant, so the B-source reads its own last state.
- The output stage and DISCH are B current sources with the engine's formulas, to the chip's own VCC and GND pins.
- Every node is shunted to ground with 1e12 ohm, the engine's gmin. The ladder holds CONT flat at 2/3 of the supply behind 3.3 kohm, where gmin moves it by 1.4e-8 V. A flat column has no span, so its bound is the 1 nV floor, and the shunts put the same gmin on both sides. Decks without a chip have no shunts.

The analysis is `tran g p stop p 0 m p`, with `g` one two-thousandth of `stop_ps` and `m` one four-hundred-thousandth. The cap `m` resolves the latch edges: ngspice converges linearly in it, and at 10 us its period is 2.4e-5 under the closed form of the same behaviour. That run has about 400000 points. `write edges.raw` keeps the latch nodes at ngspice's own points, and `linearize` puts every other saved vector on the `g` grid, 2001 points, before `write tran.raw`. The script takes each 0.5 crossing of a latch node, linear between ngspice's points and rounded to the picosecond, as an edge of that chip's `out`.

## Skips

An example is skipped when a part has no `kind`, when it has a kind the script does not translate, when a part is an `atmega328p`, which runs firmware that a deck has no CPU for, when a part has no `pins` or is missing a pin, when two names are equal ignoring case, when a net other than ground is named `gnd`, which ngspice reads as ground, when a pin names an undeclared net, when a held capacitor is in an operating point deck or has no pin on ground, when a chip is in an operating point deck, when a transient deck has a name starting with `ctl_`, `bp` or `x_`, or when `netlist.json` has a shape the engine's parser would reject. The report lists the skip and the reason in the one JSON document.

A firmware example has no ngspice reference. ngspice has no CPU to run its firmware, so it cannot say where the pin edges fall. So `examples/atmega-blink` has no `deck.cir` or `ngspice.json`, and both `--check` and `crates/cli/tests/ngspice.rs` fail if an example with an `atmega328p` part has either. `crates/cli/tests/blink.rs` checks it instead. Its PB5 edges sit at the picoseconds its firmware's cycle counts give, and its LED current is the closed form of the output stage, the resistor and the LED within Newton's 1e-6.

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

`switches` is every contact edge before the stop, as the script expanded it. `t_s` is ngspice's time points in seconds, or the linearized grid in a chip deck. A chip deck adds two keys:

```json
"grid_ps": 2000000000,
"logic": {
  "#u1.logic.out": [
    { "level": "low", "t_ps": 626249605612 },
    { "level": "high", "t_ps": 953476520653 }
  ]
}
```

`grid_ps` is the linearized grid's spacing. `logic` maps each chip's `out` ref to its edges from `edges.raw`. `values` holds every net voltage and each part's `i` and `v` at those points. The ground net is 0 on every row. `p` is not in the file.

`op` holds every net voltage and each part's `i` and `v`, under the engine's refs and sign rules. Values come from ngspice's ASCII raw file, which has 16 significant digits. `p` is left out. It is `v * i` on both sides.

## The comparison

`crates/cli/tests/ngspice.rs` runs each example. The run manifest's `input.sha256` must equal `netlist_sha256`. Then every key in `op` must satisfy

```text
|engine - ngspice| <= 1e-3 * |ngspice| + floor
```

The floor is 1e-9 for a voltage and 1e-12 for a current. It only matters near 0. The 1 pA floor does not absorb ngspice's junction gmin, `gmin` times `V`, in cutoff. A reverse-biased example needs a different comparison before it is added.

A transient example is checked in four steps.

1. The run has no diagnostics, and `observations.json`'s `switches` equals the reference's `switches`. Both equal `bounce::expand` on the parsed netlist.
2. Each signal in the reference's `logic` has the same levels in the same order as the engine's `logic.vcd`. The mean time between falling edges is within 1e-3 of ngspice's, and each edge is within 1e-3 of that period of its partner, so the phase is checked as well as the mean.
3. The reference is read as piecewise linear between its points. It is scored at the engine's rows, without the rows inside two kinds of window. The first is the open interval from each switch edge or source jump to 1 ns later, even when that time is after `stop_ps`. A stop inside the interval is mid-ramp and is not scored. The jumps come from the parsed netlist, so `ngspice.json` has no key for them. A row at the edge holds the old contact or level. Every step after it is stamped with the new one. ngspice ramps the switch or the source across that nanosecond, so the interiors are not comparable. The second is around each pair of chip edges, from the earlier minus `grid_ps` to the later plus `grid_ps`. Inside it the linearized reference reads a chord across the edge. The deck's 1 ns ramp shifts ngspice from an ideal jump by about `dV * 0.5 ns / tau`. The shift is 5.0e-4 V at a time constant of 1 us. A jump into a time constant under about 1 us is not held to the 1e-3 bound against ngspice.
4. At those rows, every trace column must satisfy

```text
max |engine - ngspice| <= 1e-3 * span + floor
```

The test derives each part's `p` as `v * i` at the samples of each side, and reads the reference's series as piecewise linear. Each stored `p` must also equal that row's `v * i` within 1e-15 W. Every trace column is one of these comparisons.

`span` is the column's maximum minus its minimum in the reference. The floor is 1e-9 for a voltage, 1e-12 for a current, and 1e-15 for a power, so a constant column passes on its floor. The test prints each column's worst error and span when it fails.

Each trapezoidal engine step above the 1 ps floor holds every capacitor's local truncation error under 1e-5 of its swing plus 1 nV, starting from a consistent restart after every edge. On one mode, error accumulates over steps to under 9.5e-5 of the swing on an exponential, and the examples measure it. Where modes couple, the accumulated error on a capacitor measured 1.2e-4 of its own swing. rc-switch's rows are 1.2e-6 V from its closed form, 4.9e-7 of the 2.5 V span. A step at the floor is accepted over its tolerance, so a time constant under about 43 ps is not held to the 1e-3 bound above. [engine.md](engine.md#accuracy) gives the threshold and the measured error.

The tolerance is relative to how far each capacitor's voltage moves in its segment, not to its level, so a small swing on a large level is held to the bound like a full one. In one probe, 5 V feeds a rail through 0.47 ohm, 22 uF on the rail is held at 5 V, and a switch puts 220 ohm on the rail at 100 us. The rail sags 10.6 mV. On a 10 us grid, the row at 120 us has `#net.vrail` 7.4e-7 V from the closed form and 7.9e-7 V from ngspice, against a bound of 1.06e-5 V. At the same row `#rsrc.p` misses ngspice by 3.0e-8 W against a bound of 2.4e-7 W. The worst column is at 0.12 of its bound. A swing under about 100 uV is held to the 1 nV floor, not to its share. A 100 uV edge measures 1.2e-4 of its swing.

The comparison does not read the engine between rows. A row is a point sample, and [run-folder.md](run-folder.md#traces) says what a read between rows needs. ngspice's points are far denser than the rows, so reading it as piecewise linear adds little. In another probe, 12 V feeds a rail through 2 ohm, 4.7 uF on the rail is held at 12 V, and a switch puts 470 ohm on the rail at 50 us. The rail sags 51 mV. On a 5 us grid every column is within its bound at the engine's 101 rows. The worst is `#c1.p`, 4.2e-5 W from ngspice against a bound of 3.1e-4 W.

The engine and ngspice agree to about 3e-7 relative on the LED examples. That gap is consistent with ngspice 42 using older values of `k` and `q` than the exact SI ones. The engine puts its 1e-12 S gmin from every node to ground, and ngspice puts it across each junction. On the divider that difference is about 4e-9 relative. A chip deck shunts every node with the same gmin.

## The astable

`examples/ne555-led` adds checks against the datasheet and against the closed form of the modelled behaviour, in `crates/cli/tests/ne555.rs`. With TRIG and THRES on the capacitor, each half period is an RC charge or discharge between 1/3 and 2/3 of the supply. Set, DISCH is off, and the capacitor charges toward `V * Roff / (RA + Roff)` through `RB + RA || Roff`. Reset, it discharges toward `V * Ron / (RA + Ron)` through `RB + RA || Ron`. The steady period is within 1e-4 of the sum of the two, and the first high time, from 0 V, within 1e-4 of its own.

SLFS022K eq. 3 and 4 give the period as `(RA + 2 RB) C / 1.44`. This table is what that formula leaves out, for the committed 10 kohm, 47 kohm and 10 uF, as a share of its 0.72222 s:

| Term | Modelled | Size |
| --- | --- | --- |
| 1.44 in place of 1/ln 2 | yes | -0.187% |
| Discharge asymptote above 0, `V * Ron / (RA + Ron)` | yes | +0.183% |
| Discharge resistance, `RA \|\| Ron` in series with RB | yes | +0.018% |
| Discharge off-state leakage, 250 Mohm | yes | +0.004% |
| THRES input current, 30 nA typ | no | 0.013%, from 1.7 mV high and 1.4 mV low |
| TRIG input current, 0.5 uA typ at 0 V | no | 0.225%, from 28.5 mV high and 23.5 mV low |
| RESET current, -0.4 mA typ | no | 0, since RESET is on the supply |

An input current moves the capacitor's asymptote by the current times the resistance it sees, which the table gives in volts for each half period. Its sign depends on the current's direction. The test's bound is the sum of the modelled terms' sizes, 0.393%. The engine's period is +0.016% from the formula. The bound is a property of the example: other resistors and capacitors give other terms, and the test computes them from the netlist. A chip on a bench adds the input currents, 0.24% more for these values.

The OUT edges are located, not stepped. Every edge is an accepted step at its picosecond, past its threshold by more than 0 and at most 1e-8 V, while the step before is not past it. The test reads those steps with a row at every step. A step-end edge would be about 20 mV past. At a tenth of the committed `max_step_ps` and at ten times it, with `grid_ps` held at the committed value, every edge moves by no more than the bound the runs' own error gives: in each half period, the largest gap between THRES and the closed form started at the run's own edge, plus the edge step's distance from the nominal threshold, over the closed form's slope, summed over the half periods before the edge. Between 1 ms and 10 ms steps, the tenth edge moves 103 us against a 106 us bound.

## Limits

Watches are read at accepted steps. A comparator input that crosses its threshold and crosses back inside one step is missed. The error control keeps steps short where a waveform curves, and `max_step_ps` and `grid_ps` cap them.
