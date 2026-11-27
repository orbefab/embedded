# ngspice references

Each example has an ngspice result next to its netlist, except `packages/engine/tests/circuits/npn-blinker`, `packages/engine/tests/circuits/atmega-analog-serial`, `packages/engine/tests/circuits/atmega-blink`, `packages/engine/tests/circuits/atmega-board-blink`, `packages/engine/tests/circuits/atmega-board-button`, `packages/engine/tests/circuits/atmega-board-knob`, `packages/engine/tests/circuits/atmega-button`, `packages/engine/tests/circuits/atmega-cst816`, `packages/engine/tests/circuits/atmega-dht22`, `packages/engine/tests/circuits/atmega-fade`, `packages/engine/tests/circuits/atmega-i2c-scan`, `packages/engine/tests/circuits/atmega-serial-echo`, `packages/engine/tests/circuits/atmega-ws2812`, `packages/engine/tests/circuits/spi-replay`, `packages/engine/tests/circuits/st77916-replay`, `packages/engine/tests/circuits/ws2812-replay`, any other example with an `atmega328p`, `spi_replay`, or `pulses_replay` part, and every example with an `op_amp` part, which [Skips](#skips) covers. A DC example has an operating point, and a transient example has a waveform. A Rust test compares the engine with it. The Rust tests never run ngspice.

```text
packages/engine/tests/circuits/<name>/
  netlist.json
  deck.cir
  ngspice.json
```

A firmware example has no `deck.cir` and no `ngspice.json`, and neither does `packages/engine/tests/circuits/npn-blinker`.

`packages/engine/tests/circuit/fixtures/<name>/` holds circuits the reference tests run, in the same layout. They are not examples. `diode-off` holds a diode with its anode at 5 V and its cathode at 8 V. `led-off` holds an LED with both pins at 5 V. `diode-reverse` holds a diode with 0.3 V of reverse bias, and `led-reverse` an LED with 0.5 V. `npn-off` and `pnp-off` hold a transistor off with its internal nodes away from 0 V. `npn-held-off` holds an NPN off with its base 0.13 V below its emitter and its collector at 12 V behind 100 ohm. `pnp-held-off` is the PNP mirror. `npn-reverse` and `pnp-reverse` hold the base-collector junction 0.13 V reverse, with the base-emitter junction at 0 V. `npn-nf-nr` and `pnp-nf-nr` set `NF` to 1.5 and `NR` to 2. The emitter is 0.13 V from the base and the collector is 0.27 V from the base, so `vbe` is below `-3 * NF*VT` and above `-3 * NR*VT`, and `vbc` is below `-3 * NR*VT`. Each of those six has an `IS` of 1e-8 A. `nmos-off` holds an `nmos` off, so its drain current is the drain-source gmin. `pmos-off` is that circuit with the supplies negated, so the drain current is the negated gmin. `npn-slow-edge` switches a transistor through 1 Mohm, so its edges are slow next to the deck's ramp and the junction capacitors set them. The example `packages/engine/tests/circuits/npn-pulse` is a different circuit, a 1 kHz pin with 100 nF on the collector. `nmos-pulse` drives an `nmos` gate through 1 Mohm, so the gate capacitances set the edge. `nmos-regions` steps the drain through reverse, linear and saturation. `pmos-regions` is that circuit with the supplies negated. `nmos-follower` holds the source off ground, with the drain at 12 V, the gate at 5 V, and 1 kohm from the source to ground.

`p-relay-npn-bare` is a 5 V relay coil switched by the 2N3904 `npn` of `packages/engine/tests/circuits/npn-led`, with no diode across the coil. A pin through 1 kohm drives the base high from 5 ms to 30 ms. The contact feeds the 220 ohm LED load of `packages/engine/tests/circuits/relay-switch`. The run stops at 60 ms on a 50 us grid with a 1 ms maximum step. The coil rings with the transistor's junction capacitances at about 9 us, and that ring sits on the 50 us grid, so the rows are not compared. The directory contains `rows-skipped`. `packages/engine/scripts/ngspice_refs.py` and `packages/engine/tests/circuit/runs/ngspice.rs` both read that file, and a fixture without it keeps its rows. `packages/engine/tests/circuit/runs/relay.rs` checks three numbers in `ngspice.json`. The envelope peak of `#net.k` and the envelope minimum of `#net.k` each stay within 1e-3 of ngspice's peak. The contact's release stays within the relay edge rule, on ngspice's own points around that edge. The model has no breakdown. A real 2N3904 breaks down far below this peak. [netlist.md](netlist.md) says a junction does not break down and `VCEO` is not checked.

## Regenerate

The script needs ngspice 42 on `PATH` and uv. On Debian and Ubuntu, `apt install ngspice` installs it.

```sh
uv run packages/engine/scripts/ngspice_refs.py                    # every example, then every fixture
uv run packages/engine/scripts/ngspice_refs.py packages/engine/tests/circuits/led-9v    # one example
uv run packages/engine/scripts/ngspice_refs.py --check            # exit 1 on drift or an unchecked reference
```

`scripts/check.sh full` runs that `--check`. The engine it runs is release `orbe-run` in cargo's target directory, so `CARGO_TARGET_DIR` selects the binary.

Stdout is one JSON document. It lists the examples written or checked, the examples skipped with a reason, and with `--check` the files that drifted. For an example with a chip, `--check` also builds and runs the engine, reads the run's `traces/logic.vcd` back with the standard library, summarises each signal, and compares that with `observations.logic`. Its `logic` list holds one entry per such example, and one that does not match exits 1. A skipped example that already has `deck.cir` or `ngspice.json` is a failure under `--check`. Its entry's `failure` names them, as in `committed ngspice.json was not checked`, and the process exits 1. A skip with neither file exits 0. `packages/engine/scripts/tests/test_ngspice_refs.py` checks those paths, each source spelling's deck line, the bounce generator's test vectors, the VCD reader and its summary, and the operating-point refusal below. It also checks the bare flyback summary. That summary keeps a collector peak, a collector minimum and a release edge, and it does not keep the rows.

A generation and `--check` both exit 1 when ngspice's log shows the operating point did not converge on the plain iteration. The message names the example and the lines ngspice-42 prints for that, `Starting dynamic gmin stepping`, `Dynamic gmin stepping failed`, `Starting true gmin stepping`, `True gmin stepping failed`, `Starting spice3 gmin stepping`, `spice3 gmin stepping failed`, `Starting source stepping`, `source stepping failed`, `Transient op started`, `Transient op failed`, and `The operating point could not be simulated successfully.` `packages/engine/scripts/tests/test_ngspice_refs.py` expects that refusal from `npn-led` with its pin at 0 V, from the same circuit as a transient, and from a pin at 1e-12 V whose dynamic gmin step completes. Without ngspice-42 on `PATH`, the regulator deck and the operating-point cases in that file fail.

Run it after any change to a `netlist.json`. The Rust test fails until you do.

## deck.cir

The script writes `deck.cir` from `netlist.json`. Do not edit it. `ngspice -b deck.cir` in that directory reruns it and writes `op.raw`, or `tran.raw` for a transient, and `edges.raw` too for a chip.

Parts are written in name order. Names are lowercased, and ground is node `0`. A resistor is `r<name>`, a voltage source is `v<name>`, a current source is `i<name>`, a capacitor is `c<name>`, a diode or LED is `d<name>` with its own `.model`, a bipolar transistor is `q<name>` with its own `.model`, and an `nmos` or a `pmos` is `m<name>` with its own `.model`. A potentiometer is two resistors, `r<name>_a` from `a` to `wiper` of `position * resistance`, and `r<name>_b` from `wiper` to `b` of `(1 - position) * resistance`. It has no `#<name>.i` and no `#<name>.v`. A `steps` position with no events is the fixed `initial`. A position with events is skipped: a moving potentiometer has one position in a deck. Numbers are written so they read back to the same f64.

A current source is `i<name> <n> <p> dc <current>`, its pins swapped. ngspice's current flows from the first node through the source to the second, and the engine's leaves at `p`. Both an operating point deck and a transient deck save `@i<name>[current]`, the current the source drives, and the script negates it into the engine's `#<name>.i`.

A transistor is written behind two 0 V ammeters, under names starting with `x_<name>`:

```text
vx_q1_c c x_q1_c dc 0
vx_q1_b b x_q1_b dc 0
qq1 x_q1_c x_q1_b 0 qmod_q1
.model qmod_q1 npn (is=8.095e-15 bf=173.2 br=0.2704 nf=1.0 nr=1.0 vaf=100.0 rb=5.707 rc=1.531 re=0.867 cje=8e-12 cjc=4e-12 mje=0 mjc=0)
```

The ammeters' currents are the engine's `#<name>.ic` and `#<name>.ib`, and the nodes between them and the device are not compared. `p` is `(V(collector) - V(emitter)) * ic + (V(base) - V(emitter)) * ib` from the saved values, as the engine computes it. `mje=0` and `mjc=0` make `cje` and `cjc` constant, as the engine's are. When the netlist sets `forward_knee_current`, the model card adds `ikf=<value>`. Every model parameter the netlist does not name keeps its ngspice default, which is the engine's equation. A deck with a transistor reserves net and part names starting with `x_`, in an operating point deck too. Such a name skips the example.

An `nmos` is written the same way, with the bulk node equal to the source and `l=100u w=100u` on the device line. `l` and `w` are equal, so `kp` alone sets the current. The card is level 1 with `is=0`, which leaves no body diode. With `tox` unset, ngspice-42's level 1 has no Meyer capacitance. The three capacitances are plain capacitors on the device side of the ammeters:

```text
vx_m1_d d x_m1_d dc 0
vx_m1_g g x_m1_g dc 0
mm1 x_m1_d x_m1_g 0 0 mmod_m1 l=100u w=100u
.model mmod_m1 nmos (level=1 vto=1.08 kp=0.0468 lambda=0.0 is=0)
cm1_gs x_m1_g 0 5.5e-11
cm1_gd x_m1_g x_m1_d 5e-12
cm1_ds x_m1_d 0 2e-11
```

The drain ammeter is `#<name>.id`. The gate ammeter is not an engine column. A transient reference keeps it under `tran.gate`, keyed by `#<name>`, and the power check and the gate current check read it there. `p` is `(V(drain) - V(source)) * id + (V(gate) - V(source)) * ig`. At an operating point the capacitors are open, `ig` is 0, and `p` is `(V(drain) - V(source)) * id`. A `pmos` is the same deck with `.model ... pmos` and `vto` negated. The netlist stores that threshold positive.

An inductor is its series resistance and its inductance, under the name `x_<name>` for the node between them:

```text
rx_l1_s vin x_l1 70.0
ll1 x_l1 0 0.505
```

That is the engine's topology line for line. ngspice's `L` is a short at the operating point, as the engine's is. The deck saves `i(l<name>)`, the engine's `#<name>.i`, and does not compare the node between the two. The deck sets no `ic=` and no `uic`, because the engine has no initial current.

A relay is written in a transient deck only. Its coil is an inductor's two lines, with a 0 V ammeter `vx_<name>_am` on its `coil_b` end. A B-source puts the magnitude of the ammeter's current on node `x_<name>_m`. Each side of the contact is an `sw` switch on that node, with `vt` halfway between the two currents and `vh` half their difference, so it turns on above `pull_in_current` and off below `drop_out_current`. The NC switch swaps the two resistances.

```text
rx_k1_s vcc x_k1 70.0
lk1 x_k1 x_k1_c 0.505
vx_k1_am x_k1_c coil dc 0
bx_k1_m x_k1_m 0 v = abs(i(vx_k1_am))
sx_k1_no vcc no x_k1_m 0 swno_k1
.model swno_k1 sw (vt=0.030357142857142857 vh=0.02321428571428571 ron=0.1 roff=100000000.0)
sx_k1_nc vcc nc x_k1_m 0 swnc_k1
.model swnc_k1 sw (vt=0.030357142857142857 vh=0.02321428571428571 ron=100000000.0 roff=0.1)
```

The ammeter's current is `#<name>.i`, and `v` is from `coil_a` to `coil_b`. The script takes the contact's edges from that current at ngspice's own points, before the reference is put on the engine's grid. Two watches read the signed current, one per sign, as the engine's do. A watch closes when its current rises above `pull_in_current` and opens when it falls to `drop_out_current` or below. The contact is closed while either watch is high. Each crossing is linear between those points and rounded to the picosecond. A raw interval that changes sign can hold the departing sign's drop-out and the arriving sign's pull-in. The edges are the comparison's contact, so an interval that reverses through zero is not taken from that switch.

A deck with a transistor also shunts every net but ground to ground with 1e12 ohm, `rx_gmin_<net>`. The engine puts its node gmin on every net, so the shunts give ngspice the engine's. Inside the transistor both put gmin in the same places, and the shunts play no part there. Each puts it across each junction. ngspice-42's default substrate junction, vertical for an `npn` and lateral for a `pnp`, adds gmin from ground to `c'` in an NPN and to `b'` in a PNP, and the engine stamps the same term. The engine puts no node gmin on a bipolar transistor's internal nodes. An off NPN with 5 V on its collector takes 10 pA in both, 5 pA across `b'` to `c'` and 5 pA from `c'` to ground, and the two agree to 7e-6 relative. The fixtures `npn-off` and `pnp-off` are within 0.009 of their bound. The three transistor examples agree to at most 3.2e-7 relative, the gap the LED examples show. An `nmos` or a `pmos` has no internal node. Its device `GMIN` is only from drain to source, inside the stamp, and the shunts put the engine's node gmin on the external nets. An off MOSFET with 5 V from drain to source takes 5 pA into the drain in both. `nmos-off` is that circuit, and `pmos-off` is the same current negated. A deck with a diode or an LED shunts every net but ground the same way, in either analysis. A deck with a zener does too, and it reserves names starting with `x_` in either analysis. The zener is two diodes and two resistances. `rx_<name>_s` is `series_resistance` from the anode to `x_<name>`. `dx_<name>_f` is the forward diode from `x_<name>` to the cathode, with the part's `Is` and `N` and no series resistance of its own. `rx_<name>_b` is `breakdown_resistance` from `x_<name>_b` to the cathode. `dx_<name>_b` is the breakdown diode from `x_<name>_b` to `x_<name>`. The script derives that diode's `Is` and `N` with the formulas in [netlist.md](netlist.md#zener). `#<name>.i` is the current in `rx_<name>_s`. The deck does not shunt `x_<name>` or `x_<name>_b`. Each diode adds gmin across its junction, and the engine stamps `GMIN` once per junction.

A linear regulator is the branch in [netlist.md](netlist.md#linear-regulator). `bx_<name>_v` sets the voltage of `x_<name>` from ground. `vx_<name>` reads the pass current from `x_<name>` to `out`, and that current is `-#<name>.iout`. `fx_<name>` returns the same current from `in` to ground. `bx_<name>_g` sets the ground current into `x_<name>_g`, and `vx_<name>_g` reads it from `in` to that node. `#<name>.iin` is the sum of the two ammeters. The deck does not shunt `x_<name>` or `x_<name>_g`. It does shunt every external net but ground. A deck with a linear regulator reserves names starting with `x_` in either analysis. `p` is `(V(in) - V(gnd)) * iin + (V(out) - V(gnd)) * iout`.

A source with a waveform writes `dc <initial>` in an operating point deck, the value the engine's operating point holds. A transient deck is described below.

The options are `temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12`. 25 C with `tnom` equal to `temp` matches the engine's 298.15 K with no temperature scaling. The tolerances are tighter than ngspice's defaults, so ngspice's own error stays far under the comparison bound.

## Transient decks

A switch is `s<name>`, a voltage-controlled switch with its own `sw` model, `vt=0.5 vh=0` and the part's `on_resistance` and `off_resistance`. A control source `vctl_<name>` drives it with a PWL waveform. The script expands the switch's events and bounce with the same integer generator as the engine, and each contact edge becomes a 1 ns ramp that starts at the edge's exact picosecond. ngspice has no ideal switch, so the ramp is the smallest change the comparison has to step around.

A source waveform is `dc <initial> pwl(...)`. Its PWL starts at `0p <initial>`, and each jump at `J` adds `Jp <old>` and `(J+1000)p <new>`, a 1 ns ramp like a contact's. A `steps` source is that same PWL, one jump per event, from `initial` to each event's `level`. The knots stop after the first one at or past `stop_ps`. The deck never writes PULSE: ngspice replaces a zero rise or fall with the analysis's time step, so a PULSE edge would ramp over `stop_ps / 1000`, not 1 ns.

Each ramp corner, and each waveform knot in (0, `stop_ps`], also gets a source `vbp<k>` with a PWL corner one picosecond later. It makes that time a breakpoint, because ngspice skips a PWL corner it lands on an ulp early.

A deck with an inductor or a relay writes each corner as its own source, `vbp<k> bp<k> 0 pwl(<corner>p 0 <stop>p 0)`. ngspice-42 keeps one pending breakpoint per PWL source. At each accepted point within its minimum breakpoint gap of the pending one, or past it, the source sets its first point after the current time as the next breakpoint. So a source's first point is a breakpoint from t = 0. A later point becomes one only after the time has passed the point before it, and a step that crosses both never stops at the later one. So a source with a second corner can lose it. At `reltol=1e-9`, a corner 1 ps later stops ngspice on `packages/engine/tests/circuits/rl-step` at 1 ms with "Timestep too small". One source per ramp, its start and end as the first two points, misses the end of the 30 ms ramp on a coil switched off into a diode. One source per corner lands every corner on both. The script reads the corners from the netlist's source jumps and contact edges, not from the deck, and refuses an inductor deck where ngspice has no point at one. It names the missing corners. A chip deck's waveform raw is linearized onto a grid, so a chip deck is checked on its edge raw, which keeps ngspice's own points.

A deck with an inductor or a relay also sets `reltol=1e-5` and caps ngspice's step at `stop_ps / 60000`. At `reltol=1e-9` ngspice crawls on a coil whose current sits near zero, and at `1e-6` and `1e-7` it can stop at a transistor's turn-off. At `1e-5` the cap holds ngspice's own error far under the bound. That makes 60000 points or more, so the reference stores its waveforms on the engine's grid, as a run with more than 3000 points does. A deck with no inductor and no relay keeps every setting above, and its reference stays byte for byte. A linear regulator does not change them.

The analysis is `tran s p stop p 0 s p` with `s` one thousandth of `stop_ps`. ngspice picks its own points under that cap, which does not depend on the engine's `max_step_ps` or `grid_ps`. The options are the same as for an operating point, apart from the inductor lines above.

A run with more than 3000 points, and no chip, stores its waveforms on the engine's grid. The deck is unchanged. Each row's sample is ngspice's waveform read in picoseconds at that row, as the comparison reads ngspice's own points, so every row scores the same error either way. A row on a jump reads ngspice's point there, which is the level from before the 1 ns ramp. When ngspice lands that point one rounding unit before the jump, the row reads a few millionths of the way up the ramp, on both sides alike. The file also stores `span`, each scored column's maximum minus its minimum over ngspice's own points, so a swing between rows still sets the bound. The derived `p` columns score the product of two stored samples, where ngspice's own points give the line through their products. On the transistor examples that moves a `p` column's worst error by under 0.04 of its bound. The largest reference that keeps ngspice's points is the `nmos-pulse` fixture, at 2745.

A capacitor with `series_resistance` (either direct ohms or a `tan_delta` and `frequency` pair, converted as `tan_delta / (2 * pi * frequency * capacitance)`) splits into `rx_<name>_s` from `a` to `x_<name>` and `c<name>` from `x_<name>` to `b`, reserving names starting with `x_` and shunting `x_<name>` when the deck adds node-gmin shunts. A capacitor with an `initial` voltage adds `.ic v(<node>)=<initial>` for the pin that is not ground, negated when that pin is `b` (or `.ic v(x_<name>)=<initial>` when `series_resistance` is set and `b` is ground). Without `uic`, ngspice holds the node there while it solves the operating point, then releases it. The node's voltage at t = 0 then matches the engine's. Its current does not. ngspice reports the capacitor's current at t = 0 as 0, and the engine reports the current the hold takes. So the transient comparison drops the t = 0 sample of that capacitor's `i` and `p` from both series before it reads them as piecewise linear. ngspice's 0 then does not enter the line the engine's first rows are compared against. Those columns are scored from the reference's next sample on, since nothing before it says what ngspice's current was. On a linearized grid that is `grid_ps`. A capacitor with `"initial": null` writes no line.

A switch, a held capacitor, a relay and a chip are translated only for a transient analysis. A held capacitor needs exactly one pin on ground. A transient deck reserves net and part names starting with `ctl_`, `bp` or `x_`. A deck with a transistor or an inductor reserves names starting with `x_` in an operating point deck too. Each of these skips the example.

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
- Every node is shunted to ground with 1e12 ohm, the engine's gmin. The ladder holds CONT flat at 2/3 of the supply behind 3.3 kohm, where gmin moves it by 1.4e-8 V. A flat column has no span, so its bound is the 1 nV floor, and the shunts put the same gmin on both sides. A deck with a diode or an LED shunts every node the same way. A deck with no chip, no transistor, no diode and no LED has no shunts.

The analysis is `tran g p stop p 0 m p`, with `g` one two-thousandth of `stop_ps` and `m` one four-hundred-thousandth. The cap `m` resolves the latch edges: ngspice converges linearly in it, and at 10 us its period is 2.4e-5 under the closed form of the same behaviour. That run has about 400000 points. `write edges.raw` keeps the latch nodes at ngspice's own points, and `linearize` puts every other saved vector on the `g` grid, 2001 points, before `write tran.raw`. The script takes each 0.5 crossing of a latch node, linear between ngspice's points and rounded to the picosecond, as an edge of that chip's `out`.

## Skips

An example is skipped when a part has no `kind`, when it has a kind the script does not translate, when a part is an `atmega328p` or an `esp32s3`, which runs firmware that a deck has no CPU for, when a potentiometer's `position` has events, when a part has no `pins` or is missing a pin, when two part names, two net names or two device names are equal ignoring case, when a net other than ground is named `gnd`, which ngspice reads as ground, when a pin names an undeclared net, when a held capacitor is in an operating point deck or has no pin on ground, when a chip is in an operating point deck, when a transient deck has a name starting with `ctl_`, `bp` or `x_`, when a deck with a transistor, an inductor, a zener or a linear regulator has a name starting with `x_`, when the example directory is named `npn-blinker`, or when `netlist.json` has a shape the engine's parser would reject. The report lists the skip and the reason in the one JSON document.

A firmware example has no ngspice reference. ngspice has no CPU to run its firmware, so it cannot say where the pin edges fall. So `packages/engine/tests/circuits/atmega-analog-serial`, `packages/engine/tests/circuits/atmega-blink`, `packages/engine/tests/circuits/atmega-board-blink`, `packages/engine/tests/circuits/atmega-board-button`, `packages/engine/tests/circuits/atmega-board-knob`, `packages/engine/tests/circuits/atmega-button`, `packages/engine/tests/circuits/atmega-cst816`, `packages/engine/tests/circuits/atmega-dht22`, `packages/engine/tests/circuits/atmega-fade`, `packages/engine/tests/circuits/atmega-i2c-scan`, `packages/engine/tests/circuits/atmega-serial-echo`, `packages/engine/tests/circuits/esp32s3-adc`, `packages/engine/tests/circuits/esp32s3-blink`, `packages/engine/tests/circuits/esp32s3-dualcore`, `packages/engine/tests/circuits/esp32s3-echo`, `packages/engine/tests/circuits/esp32s3-encoder`, `packages/engine/tests/circuits/esp32s3-hello`, `packages/engine/tests/circuits/esp32s3-pattern`, `packages/engine/tests/circuits/esp32s3-pulls`, `packages/engine/tests/circuits/esp32s3-ring`, `packages/engine/tests/circuits/esp32s3-screen`, `packages/engine/tests/circuits/esp32s3-timers`, `packages/engine/tests/circuits/esp32s3-touch` and `packages/engine/tests/circuits/esp32s3-uart` have no `deck.cir` or `ngspice.json`, and both `--check` and `packages/engine/tests/circuit/runs/ngspice.rs` fail if an example with an `atmega328p` or an `esp32s3` part has either. `packages/engine/scripts/esp32s3_blink.py` checks the ESP32-S3 blink example, `packages/engine/scripts/esp32s3_hello.py` the hello example, `packages/engine/scripts/render_st77916.py --pattern` the pattern example, `packages/engine/scripts/esp32s3_screen.py` the screen example, `packages/engine/scripts/esp32s3_dualcore.py` the dualcore example, `packages/engine/scripts/esp32s3_echo.py` the echo example, `packages/engine/scripts/esp32s3_ring.py` the ring example, `packages/engine/scripts/esp32s3_pulls.py` the pulls example, `packages/engine/scripts/esp32s3_encoder.py` the encoder example, `packages/engine/scripts/esp32s3_adc.py` the adc example, `packages/engine/scripts/esp32s3_timers.py` the timers example, `packages/engine/scripts/esp32s3_touch.py` the touch example and `packages/engine/scripts/esp32s3_uart.py` the uart example, as [esp32s3.md](esp32s3.md#the-examples) says. `packages/engine/tests/circuit/runs/blink.rs` checks the blink example. Its PB5 edges sit at the picoseconds its firmware's cycle counts give, and its LED current is the closed form of the output stage, the resistor and the LED within Newton's 1e-6. `packages/engine/tests/circuit/runs/button.rs` checks the button example. PD2 follows the switch, PB5 follows it a synchronizer and a loop later, and the LED current is that same closed form. `packages/engine/tests/circuit/runs/analog_serial.rs` checks the analog-serial example. The wiper voltage is the potentiometer's closed form with the pin's leakage, within the divider's 1e-8, and TXD is 8N1 frames of the code §24.7 gives for that voltage. `packages/engine/tests/circuit/runs/fade.rs` checks the fade example. Its PB1 edges sit at the picoseconds its firmware's cycle counts and docs/engine/avr.md parameter 4 give, and its LED current is that same closed form. `packages/engine/tests/circuit/runs/serial_echo.rs` checks the serial-echo example. Each echo in `traces/serial.csv` is the byte the host sent, and it starts after that frame's `RXC0` and within the vector plus one bit. PB5 rises after `'1'` and falls after `'0'`, and the LED current is that same closed form. The same netlist with the host at 115200 never raises PB5. `packages/engine/tests/circuit/runs/board.rs` checks `packages/engine/tests/circuits/atmega-board-blink`. Every `atmega-board` example keeps the fragment's parts by name, apart from four input fields. The LED current and the idle D0 voltage match their closed forms. A DTR fall and a reset-button release each return PC6 within 200 ns of the RC closed form, and PB5's next rise is within one cycle of 65.0154375 ms after that release. The DTR rise clamps RESET within 2 mV of 6.002 V and the diode within 2 mA of 110 mA, with no rating warning. With `packages/engine/tests/firmware/pullup.hex`, PB5 reads low. With `packages/engine/tests/firmware/serial_echo.hex`, each byte in `traces/serial.csv` is the byte the host sent. `packages/engine/tests/circuit/runs/board.rs` also checks `packages/engine/tests/circuits/atmega-board-button`. The pin 13 LED follows `sw1`. That file sets `usb_dtr` to 0 V on the blink fragment after the session has advanced, and checks that same release. `packages/engine/tests/circuit/runs/knob.rs` checks `packages/engine/tests/circuits/atmega-board-knob`. The sketch prints the counts its script of detents gives, and a session that applies those detents live reads the logic edges and the serial rows `orbe-run run` writes for the example. `packages/engine/tests/circuit/runs/i2c.rs` checks `packages/engine/tests/circuits/atmega-i2c-scan` and the fixtures `i2c-scan-400k`, `i2c-stretch`, `i2c-arbitration`, `i2c-contend` and `i2c-cst816-fast`, which run firmware too and have no reference. [I2C](#i2c) says what it checks. `packages/engine/tests/circuit/runs/cst816.rs` checks `packages/engine/tests/circuits/atmega-cst816`. The sketch prints the two points, `packages/engine/scripts/i2c_decode.py` matches every row of `traces/i2c.csv`, `INT` pulses for 100 us at each touch, and a session that applies those points live reads the logic edges and the serial rows `orbe-run run` writes for the example. The same file checks `i2c-cst816-fast`, an ATmega328P at 20 MHz with `TWBR` 0 that reads register `0xA7` and ends that read with a NACK. `packages/engine/tests/circuit/runs/dht22.rs` checks `packages/engine/tests/circuits/atmega-dht22`. The sketch prints the world the read before measured, and the `DATA` edges the sensor's watch saw carry each read's five bytes.
`packages/engine/tests/circuits/spi-replay` has no reference either. The script does not translate a `spi_replay`, because a deck has no core to send its transactions, so it skips the example. `--check` fails if a skipped example has `deck.cir` or `ngspice.json`, and `packages/engine/tests/circuit/runs/ngspice.rs` fails if an example with a `spi_replay` part has either. `packages/engine/tests/circuit/runs/link.rs` checks its `traces/link.csv` against the rows its events give.

The `op-amp-*` examples have no reference. The script does not translate an `op_amp`, so it skips them. A deck could only restate the engine's own law as behavioural sources, which checks the integration every other example already checks and not the law. `packages/engine/tests/circuit/runs/op_amp.rs` checks the law instead, against the datasheet's bands and against the law solved by hand for each circuit, as [netlist.md](netlist.md#op-amp) says. `packages/engine/tests/circuit/runs/ngspice.rs` fails if an example with an `op_amp` part has `deck.cir` or `ngspice.json`.

`packages/engine/tests/circuits/st77916-replay` has a `spi_replay` part, so the script skips it too. [ST77916](#st77916) says what checks it.

`packages/engine/tests/circuits/atmega-ws2812` has an `atmega328p`, so it has no reference. `packages/engine/tests/circuit/runs/ws2812.rs` checks it. The chain's colours are the 13 the sketch sends, green `i`, red `i + 16` and blue `i + 32` for LED `i`, and `packages/engine/scripts/ws2812_decode.py` matches them from `traces/logic.vcd`. The script does not run the engine's decode. It cuts `DIN` into runs of one level, makes a bit of each high by its width alone, and ends a frame at each low longer than 280 µs. It shares no state machine with the engine. `packages/engine/scripts/tests/test_ws2812_decode.py` decodes a made-up line whose power-on low is longer than 280 µs, which ends no frame. `packages/engine/tests/circuits/ws2812-replay` has a `pulses_replay`. The script does not translate that kind, so it skips the example. The same test checks that its colours match the bit-bang, and that the replay split into two events, or held at an idle high, writes the same `leds.csv`. It also splits a 2-LED frame into two events at every pulse boundary, on either idle level, 0 ps or 1 ps apart, and stops the run around each event and each latch. Each split folder's `leds.csv`, `observations.json` and `diagnostics.json` equal those of one event holding the frame with that gap. On an idle-high line, a split inside the reset low does not. The return to idle ends the first low, and the next record's low starts the reset again, so that frame never latches or latches later than the one record. A low the line was already holding starts the reset at the record's start. `packages/engine/tests/circuit/runs/ngspice.rs` fails if either example has `deck.cir` or `ngspice.json`.

`packages/engine/tests/circuits/npn-blinker` has no `deck.cir` or `ngspice.json` either. The supply steps from 0 V to 5 V at 1 ns. ngspice aborts the script's deck at 2.001 ns on node `c1`. That deck has the 0 V ammeters, `reltol=1e-9` and `vntol=1e-12`. With `reltol=1e-9` and `vntol` left at 1e-9, and with no ammeters, ngspice aborts at 1.001 ns on `c1`. So neither `--check` nor `packages/engine/tests/circuit/runs/ngspice.rs` accepts either file. `packages/engine/tests/circuit/runs/blinker.rs` checks the example. The run has no diagnostics. Both LEDs light for about 9 ms, then take turns, and each LED's steady on time is the value in [netlist.md](netlist.md). Each falling edge of the first collector stays within 1e-4 of the period of a run at a tenth of `max_step_ps`, and within 1e-4 of the period of the recorded ngspice run. That period is within 1e-3 of the same ngspice run. The test records the deck. The deck drops the ammeters, sets `reltol` to 1e-6 and `vntol` to 1e-9, and uses a 200 us maximum step.

Some variants of `packages/engine/tests/circuits/regulator-load-step` have no reference, because ngspice-42 aborts them. Each puts a 33 ohm preload in place of the 330 ohm one on the fitted part. With 10 uF on the output and the input stepping to 3 V, 3.5 V or 4 V, ngspice stops during the input step, at 1.35624e-09 s, 1.295e-09 s and 1.16476e-09 s. With 100 uF, a 0.5 ohm dropout resistance and a 0.6 V dropout voltage, the step to 5 V stops at 1.11702e-09 s. With 1 mF, the step to 5 V runs on and stops at 0.001 s, the first load edge. The timestep is 6.25e-18 s at each stop. A release build of `orbe-run` runs all five to `end_ps` 5000000000 with no errors. None of them is an example or a fixture.

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
  "jumps": [1000000000],
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

`switches` is every contact edge before the stop, as the script expanded it. `jumps` is every source jump before the stop, in part order. The script computes each time itself. A `serial` boundary is `t_ps` plus the floor of `i * 10^12 / baud` picoseconds. `t_s` is ngspice's time points in seconds, the engine's grid when a run has more than 3000 points, or the linearized grid in a chip deck. A run on the engine's grid adds `span`, which maps every scored column to its span over ngspice's own points. Its keys are the `values` keys, each part's `p`, and each `nmos` or `pmos` gate current as `<ref>.ig`. A chip deck or a relay deck adds two keys:

```json
"grid_ps": 2000000000,
"logic": {
  "#u1.logic.out": [
    { "level": "low", "t_ps": 626249605612 },
    { "level": "high", "t_ps": 953476520653 }
  ]
}
```

`grid_ps` is the spacing of the reference's points. A chip deck uses its linearized grid. A relay deck stored on the engine's grid uses that grid. Any other deck uses ngspice's step cap. `logic` maps each chip's `out` ref to its edges from `edges.raw`, and each relay's `contact` ref to its edges from its coil current. `values` holds every net voltage and each part's `i` and `v` at those points. The ground net is 0 on every row. `p` is not in the file.

A fixture with a `rows-skipped` file stores no `tran` and no `op`. `p-relay-npn-bare` is that fixture. It stores the collector extreme and the release.

```json
{
  "collector": "#net.k",
  "minimum": -10.22129921450732,
  "peak": 3384.041225390629,
  "release": {
    "coil": "#k1.i",
    "i": [0.01041058231947246, 0.006877074114825975],
    "ref": "#k1.logic.contact",
    "span": 0.07515176667522583,
    "t_ps": 30017287835,
    "t_s": [0.03001677043193246, 0.03001732991798663]
  },
  "rows_skipped": "ringing at about 9 us sits on a 50 us grid"
}
```

`collector` is the coil node. `peak` and `minimum` are ngspice's own extremes of that voltage. `rows_skipped` is the text of the fixture's `rows-skipped` file, and it says why the rows are absent. `release.ref` is the contact. `release.t_ps` is the release in picoseconds. `release.t_s` and `release.i` are ngspice's two raw points around that edge, not grid rows. `release.coil` is the coil current. `release.span` is that current's maximum minus its minimum over the raw file. The file also keeps `format`, `netlist_sha256`, `deck_sha256`, `ngspice` and `options`.

`op` holds every net voltage and each part's `i` and `v`, under the engine's refs and sign rules. A linear regulator's keys are `iin`, `iout` and `p`. Values come from ngspice's ASCII raw file, which has 16 significant digits. `p` is left out of a two-pin part. It is `v * i` on both sides. A linear regulator stores `p`, and `iin` and `iout` are currents, with the current floor.

## The comparison

`packages/engine/tests/circuit/runs/ngspice.rs` runs each example. The run manifest's `input.sha256` must equal `netlist_sha256`. Then every key in `op` must satisfy

```text
|engine - ngspice| <= 1e-3 * |ngspice| + floor
```

The floor is 1e-9 for a voltage, 1e-12 for a current and 1e-15 for a power. It only matters near 0. A linear regulator's `p` is `(V(in) - V(gnd)) * iin + (V(out) - V(gnd)) * iout` on both sides. A diode or LED puts gmin across the junction, as ngspice does, and puts no node gmin on the internal node. A zener puts gmin across each junction the same way, and the deck above is the circuit the comparison runs. Below `-3 * n * VT` of junction voltage it follows ngspice-42's reverse law, `-Is * (1 + (3 * n * VT / (e * V))^3)`, in place of the exponential. `diode-off` and `led-off` hold the off operating point, `diode-reverse` and `led-reverse` hold the reverse law, and all four are inside the bound. Each transistor junction follows the same law below `-3 * n * VT`, with `NF` for `vbe` and `NR` for `vbc`. Below that voltage the law moves the current by at most 0.405% of `IS`. That is under the floor for the 2N3904 and 2N3906 values, so the reverse transistor fixtures take an `IS` of 1e-8 A. `npn-held-off` and `pnp-held-off` show it on the emitter supply's current, and `npn-reverse` and `pnp-reverse` on `ic` and `ib`. `npn-nf-nr` and `pnp-nf-nr` give the base-emitter junction `NF` at 1.5 and the base-collector junction `NR` at 2.

A transient example is checked in four steps.

1. The run has no diagnostics, and `observations.json`'s `switches` equals the reference's `switches`. Both equal `bounce::expand` on the parsed netlist. The reference's `jumps` equal the source jumps of the parsed netlist, picosecond for picosecond.
2. Each signal in the reference's `logic` has the same levels in the same order as the engine's `logic.vcd`. For a chip's `out`, the mean time between falling edges is within 1e-3 of ngspice's, and each edge is within 1e-3 of that period of its partner, so the phase is checked as well as the mean. A relay's `contact` edge is within the time ngspice's coil current takes to move 1e-3 of its span plus the floor, at the slope of the reference interval that holds ngspice's edge. That is the value bound read as time.
3. The reference is read as piecewise linear between its points. It is scored at the engine's rows, without the rows inside two kinds of window. The first is the open interval from each switch edge or source jump to 1 ns later, even when that time is after `stop_ps`. A stop inside the interval is mid-ramp and is not scored. Those source jumps are the reference's `jumps`. A row at the edge holds the old contact or level. Every step after it is stamped with the new one. ngspice ramps the switch or the source across that nanosecond, so the interiors are not comparable. The second is around each pair of logic edges, from the earlier minus `grid_ps` to the later plus `grid_ps`. Inside it the reference reads a chord across the edge, and a contact that moves a load's current between the two edges moves it by the whole step. The deck's 1 ns ramp shifts ngspice from an ideal jump by about `dV * 0.5 ns / tau`. The shift is 5.0e-4 V at a time constant of 1 us. A jump into a time constant under about 1 us is not held to the 1e-3 bound against ngspice.
4. At those rows, every trace column must satisfy

```text
max |engine - ngspice| <= 1e-3 * span + floor
```

The test derives each part's `p` as `v * i` at the samples of each side, and reads the reference's series as piecewise linear. Each stored `p` must also equal that row's `v * i` within 1e-15 W. Every trace column is one of these comparisons.

An `nmos` or a `pmos` adds one comparison that is not a trace column. Its gate current on each engine row is `(p - (V(drain) - V(source)) * id) / (V(gate) - V(source))`, from the stored `p`, and it is held to `tran.gate` with the 1 pA floor. A row whose gate is within 1 mV of its source does not fix that current and is not scored.

`span` is the column's maximum minus its minimum in the reference, or the reference's stored `span` when it has one. The floor is 1e-9 for a voltage, 1e-12 for a current, and 1e-15 for a power, so a constant column passes on its floor. The test prints each column's worst error and span when it fails. A fixture with a `nodes-only` file holds only its `#net.*` columns to the bound, and the test still compares and prints every column. `nmos-ring-100pf` is that fixture, a three-stage NMOS ring with 100 pF on each pin pair that times itself for 300 us. For the same edge time its drain and gate currents move about nine times as far as its nodes, in shares of their spans, and main's steps already leave them 4 to 6 times outside the bound. The file's text says so.

Each engine step above the 1 ps floor holds every capacitor's and inductor's local truncation error under 1e-7 of its range plus Newton's absolute tolerance on it, 1 nV for each end of a capacitor that is not ground, or, for 5 us after a chip's or a core's edge that moved its slope, for a device's capacitance 2e-16 C over it when that is more, up to 3e-6 of its range. On a nonlinear circuit it also allows how far the store moved on the step's last Newton iteration. The step after every edge starts from the edge's restart solve, and an edge that moves a store's slope starts a segment there. Error accumulates over steps: a 10 ns discharge after an edge measures 7.7e-6 of its swing, and a 1 MHz pin into a 10 ns load 1.0e-5. A step at the floor is accepted over its tolerance, so a time constant under about 70 ps is not held to the 1e-3 bound above. [engine.md](engine.md#accuracy) gives the threshold and the measured error.

The tolerance is relative to how far each store's state has moved since t = 0, not to its level, so a small swing on a large level is held to the bound like a full one, and a ring at 5 V takes the steps one at 0 V does. A swing under about 10 mV is held to the 1 nV floor, not to its share.

The comparison does not read the engine between rows. A row is a point sample, and [run-folder.md](run-folder.md#traces) says what a read between rows needs. ngspice's points are far denser than the rows, so reading it as piecewise linear adds little. In another probe, 12 V feeds a rail through 2 ohm, 4.7 uF on the rail is held at 12 V, and a switch puts 470 ohm on the rail at 50 us. The rail sags 51 mV. On a 5 us grid every column is within its bound at the engine's 101 rows. The worst is `#c1.p`, 4.2e-5 W from ngspice against a bound of 3.1e-4 W.

The engine and ngspice agree to about 3.2e-7 relative on `packages/engine/tests/circuits/led-5v`. That gap is consistent with ngspice 42 using older values of `k` and `q` than the exact SI ones. Both put 1e-12 S across the LED junction. The engine also puts 1e-12 S from every external net to ground. On the divider, whose deck has no such shunt, that node gmin is about 4e-9 relative. A chip deck shunts every node with the same gmin.

`packages/engine/tests/circuits/relay-switch` is within 0.0168 of its bound on every column, the worst being the coil's switched end, `#net.coil`. Its contact closes 0.153 us and opens 0.161 us before ngspice's, against bounds of 28.4 us and 32.0 us. Its reference has 1201 rows on the engine's grid from about 60000 ngspice points.

`packages/engine/tests/circuits/relay-flyback` is within 0.1334 of its bound on every column. The worst sample is `#net.coil` at 45.8 ms, 0.7577 mV against a bound of 5.679 mV. `#d1.v` and `#k1.v` have the same error. On the stored rows the coil current leads ngspice by 0.251 us at 44 ms and by 0.082 us at 45.7 ms. Each lead is the current error divided by the forward difference of the reference current on the stored grid. At 40.9 ms that lead is 0.144 us. The contact closes 206672 ps and opens 145557 ps before ngspice, against bounds of 32.7 us and 30.1 us. Under `0.19.0` it closed 335 ps and opened 190887 ps before ngspice, and this sample was 0.2198 of its bound, 1.248 mV. The close is farther from ngspice than `0.19.0` and stays inside 32.7 us. Under `0.16.0` the current led by 0.193 us at 44 ms and by 0.205 us at 45.7 ms, and this sample was 0.3494 of its bound. The shorter lead at 45.7 ms is where the ratio fell from `0.16.0` to `0.17.0`. It is not a tighter solve. ngspice finishes this deck with a step cap of 1 us, 1.5 us, 2 us or 4 us. A 0.99 us cap stops with `Timestep too small` at 53.523 ms. Setting `reltol` to 1e-6 or to 3e-6 stops the same way before the pin rises. A `trtol` of 1 stops at 53.066 ms, and `gear` stops at 46.900 ms, both with `Timestep too small` and after 45.8 ms. The run accepts 1340 steps after t = 0. Before a capacitor's flipping current took the picoampere floor it accepted 2670, this sample was 0.1228 of its bound, and the contact opened 146682 ps before ngspice. Its reference has 1201 rows on the engine's grid.

`p-relay-npn-bare` compares the envelope of `#net.k`, not the rows. The engine peak is 3384.1184 V against ngspice's 3384.0412 V, a miss of 0.0772 V inside a bound of 3.384 V. The engine minimum is -10.2208 V against ngspice's -10.2213 V, a miss of 0.000459 V inside the same bound. The contact opens 981 ps after ngspice's edge, inside a bound of 11.9 ns. Under `0.19.0` the peak missed by 0.299 V and the release by 2675 ps.

`packages/engine/tests/circuits/rl-step` also has a closed form, checked in `packages/engine/tests/circuit/transient.rs`. Its rows are at most 4.3e-7 A from `5 V / 70 ohm * (1 - exp(-t / tau))` and its decay, 6.1e-6 of the 71 mA swing, at 29.9 ms. Against ngspice its worst column is 0.0062 of its bound.

## Catalog sweep

`packages/engine/tests/circuit/catalog/<name>/` holds one to three canonical circuits per unique ngspice-translatable electrical parameter set in `parts/*/part.jsonc`, in the same `netlist.json` / `deck.cir` / `ngspice.json` layout as the examples and fixtures. `packages/engine/scripts/catalog_sweep.py` extracts each parameter set directly from `parts/` and generates the circuits:

- **Diodes and LEDs**: forward and reverse operating points (including multi-die packages such as `display-7seg-ca`, `display-7seg-cc`, `led-5mm-rgb-ca` and `led-5mm-rgb-cc`).
- **Zeners**: breakdown and forward operating points (`zener-1n4733a`).
- **BJTs**: active operating point (driving `npn-2n2222-to92` into high injection so `ikf` is exercised) and a short switching transient covering cutoff, active, saturation and junction capacitances.
- **MOSFETs**: on-state operating point and a short gate-pulse transient covering cutoff, linear, saturation, gate capacitances and gate current.
- **Linear regulators**: regulating, dropout and current-limit operating points (including `ch340g`'s internal 3.3 V regulator).
- **Potentiometers**: loaded divider operating points at positions 0.1, 0.5 and 0.9.
- **Relays**: short flyback-protected switching transient covering pull-in, drop-out and asymmetric NO/NC contact loads (`relay-srd-05vdc-sl-c`).
- **Inductors**: DC operating point and a short RL step transient (`inductor-0805`).
- **Batteries**: loaded operating points of the cell voltage and internal resistance (`battery-9v`, `battery-aa-4x`).
- **Capacitors, switches and fixed resistors**: short RC step transients (including direct-ohm and `tan_delta` ESR capacitors), switch switching transients and resistive divider operating points.

`packages/engine/scripts/catalog_sweep.py --check` verifies that every directory under `parts/` is either covered or listed in `EXCLUSIONS` with a reason, and that every committed `catalog/*/netlist.json` matches `parts/`. `packages/engine/scripts/ngspice_refs.py --check` covers `packages/engine/tests/circuit/catalog/` alongside `circuits/` and `fixtures/`, and `scripts/check.sh engine-circuits` runs both. The Rust test `every_catalog_part_matches_its_ngspice_reference_within_a_tenth_of_a_percent` in `packages/engine/tests/circuit/runs/ngspice.rs` compares every catalog circuit against its committed `ngspice.json` under the same `1e-3 * |ng| + floor` (OP) and `1e-3 * span + floor` (transient) bounds without running ngspice.

Parts that ngspice cannot judge stay out of the catalog sweep, each recorded in `EXCLUSIONS` inside `packages/engine/scripts/catalog_sweep.py`:

- **Op-amps** (`lm358-dip8`, `lmv358-msop8`): behavioural macromodels checked against datasheet bands and closed forms in `packages/engine/tests/circuit/runs/op_amp.rs`.
- **`analog_temp_sensor` and `world` curves** (`tmp36`, `ldr-5mm`, `thermistor-ntc-10k`): resistance or output curves driven by world quantities (`temperature`, `light`), which ngspice has no model for.
- **Clamped-`IS` LEDs** (`led-5mm-green`, `led-5mm-yellow`, `led-3mm-yellow`): their fitted `saturation_current` (`6.776e-64 A`, `1.779e-33 A`, `1.623e-30 A`) falls below ngspice-42's `1e-28 A` diode clamp.
- **Firmware MCUs and digital/protocol behavioral devices** (`atmega328p-*`, `esp32s3-qfn56`, `bmp280`, `dht22`, `hc595-dip16`, `hd44780-*`, `max7219-8x8`, `servo-sg90`, `sk68xxmini-hs`, `ssd1306-oled-128x64`, `ws2812b-5050`): firmware or digital protocol models with no SPICE analog equivalent.
- **Host USB/jack stubs, structural/non-simulated parts and non-battery composites**: idle voltage-source stubs (`atmega16u2-qfn32`, `barrel-jack-dc21`, `cp2102n-qfn28`, `usb-*`), join-only or non-simulated mechanical parts, and board/module composites whose translatable leaf parts are swept directly.

For ad-hoc differential testing beyond fixed canonical circuits, `uv run scripts/random_circuits.py --seed <S> --count <N> [--out DIR]` generates seeded random circuits of 2–5 nonlinear catalog parts plus passives and switches (with a DC path to ground on every node and at least 10 ohm between any source and a capacitance or MOSFET gate), compares `orbe-run` against ngspice-42 under the same bounds, counts ngspice aborts separately from engine failures, and writes minimal repros (`netlist.json`, `deck.cir`, `mismatch.json`) on failure. It is run by hand, not in CI.

## The astable

`packages/engine/tests/circuits/ne555-led` adds checks against the datasheet and against the closed form of the modelled behaviour, in `packages/engine/tests/circuit/runs/ne555.rs`. With TRIG and THRES on the capacitor, each half period is an RC charge or discharge between 1/3 and 2/3 of the supply. Set, DISCH is off, and the capacitor charges toward `V * Roff / (RA + Roff)` through `RB + RA || Roff`. Reset, it discharges toward `V * Ron / (RA + Ron)` through `RB + RA || Ron`. The steady period is within 1e-4 of the sum of the two, and the first high time, from 0 V, within 1e-4 of its own.

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

The OUT edges are located, not stepped. Every edge is an accepted step at its picosecond, past its threshold by more than 0 and at most 1e-8 V, while the step before is not past it. The test reads those steps with a row at every step. A step-end edge would be about 20 mV past. At a tenth of the committed `max_step_ps` and at ten times it, every edge moves by no more than the bound the runs' own error gives: in each half period, the largest gap between THRES and the closed form started at the run's own edge, plus the edge step's distance from the nominal threshold, over the closed form's slope, summed over the half periods before the edge. Between 1 ms and 10 ms steps, the tenth edge moves 113 us against a 116 us bound.

## I2C

`packages/engine/tests/circuits/atmega-i2c-scan` runs `packages/engine/tests/firmware/sketch/i2c_scan.hex`, a Wire scanner, on 4.7 kohm pull-ups with nothing else on the bus. Each address from 1 to 126 is a START, SLA+W with no acknowledge, and a STOP, and the sketch prints `no devices`. `packages/engine/tests/circuit/runs/i2c.rs` checks those rows of `traces/i2c.csv` and that text in `traces/serial.csv`. Every row's time is an SCL or SDA edge in `logic.vcd`, and every SCL period inside an address byte is `T`, 10 us. Its fixtures take the same chip:

- `i2c-scan-400k` runs `i2c_scan_400k.hex`, which sets 400 kHz, and every SCL period inside a byte is 2.5 us.
- `i2c-stretch` closes a switch from SCL to ground at 240 us and opens it at 290 us, across the TWI's release of SCL in address 1. SCL rises at 290 us, falls `S` later, and the byte ends later than the example's by 290 us less that release.
- `i2c-arbitration` closes a switch from SDA to ground at 285 us, before address 1's last bit, a 1, and opens it at 297 us while SCL is high. The winner is that switch. It releases SDA while SCL is high, so the byte decodes as a bus error. The TWI loses arbitration, `endTransmission` returns 4, and the sketch prints `error 4 at 0x1` and scans on from address 2.
- `i2c-contend` puts two chips running `i2c_contend.hex` on one bus. They wait to the same microsecond and send SLA+W together, `u1` to 0x50 and `u2` to 0x51. `u2` loses arbitration at the last address bit and `u1` finishes and sends the STOP. Both chips read the same SCL and SDA edges.
- `i2c-read-ack` puts those two chips and a `cst816` at 0x15 on one bus, running `i2c_read_ack.hex`. `u1`'s PD2 is grounded, so it asks for two bytes and ACKs the first. `u2` asks for one byte and NACKs it. They read the same byte. `u2` loses on that NACK, status 0x38, and `requestFrom` returns 0. `u1` reads both bytes and sends the STOP.

`packages/engine/tests/circuit/fixtures/i2c-rc-bus` has no chip. Two switches put a START, two bits and a STOP onto 4.7 kohm pull-ups with 200 pF per line, and it has a reference, as any circuit without a chip does.

`packages/engine/scripts/i2c_decode.py <run>` decodes the bus apart from the engine. It reads each `atmega328p` part's own `PC5` and `PC4` in `logic.vcd`, writes the phases, and matches each master's view against its rows of `traces/i2c.csv`, within one CPU cycle. Every chip's view must decode to the same phases. An SDA change while SCL is high during a byte is a bus error, including the first bit of the byte a START opened. The SCL high after a finished byte is a STOP or a repeated START. The decoder cannot separate bit 0 of the next data byte from those two. A bus error in that high makes it exit 1. An arbitration loss on that bit makes it exit 1 only when the winner changes SDA during that high. A master that lost arbitration matches the byte that starts at its `arbitration_lost` row. An address or a data byte matches on that time, which is the byte's first bus change. A loss needs a low read, so a bus error with none cannot be one. The read is an SCL rise on bits 7 to 0 of an address, on bits 7 to 0 of a write data byte, or on the ACK bit of a read data byte. A drawn folder whose only low read is the ACK of a write data byte is rejected. A rule that counted that bit would accept the folder. A low read does not show a loss. A decoded phase before the row is a mismatch. The phases after the loss, up to the next START, are the winner's. A START or a STOP at that time does not match. `--timing` lists each timing row of DS40002061B Table 29-14 for the measured rate, its count, its least value, and every value under the table's minimum, and exits 1 on one. The rise and fall rows are not checked. tHD;DAT's maximum is checked only on SCL lows no longer than 1.5 times the median low, because the TWI holds SCL low while `TWINT` is set. It needs Python 3.11 and no package. `cargo test` runs it through `uv` on the hand-made folders in `packages/engine/tests/circuit/decode`. The fixtures' own runs are not decoded there, because the Rust tests check the same rows, except `i2c-read-ack`, which its test writes and checks with `--timing`. Those folders make no timing claim. They are drawn traces, and `--timing` on them is not a check of Table 29-14.

An `esp32s3` run has no `traces/i2c.csv`. `packages/engine/scripts/i2c_decode.py` also reads each `cst816` part's own `SCL` and `SDA`, and each `esp32s3` part's GPIO wires on the nets a `cst816`'s `SCL` and `SDA` sit on. It opens `traces/i2c.csv` only for an `atmega328p` view. A view with no rows is compared with the others. When an `atmega328p` view is decoded, a controller view that decodes to the same phases adds no line. `--timing-s3 HZ` checks the `esp32s3` and `cst816` views against the lengths an ESP-IDF master writes at 100 kHz or 400 kHz, to the picosecond, and the `cst816`'s 300 ns hold. It reads IDF's register values the way the model does, five of them as their register plus one, so it is not an independent check of that reading. It does not apply Table 29-14 to those views. `packages/engine/tests/circuits/esp32s3-touch` is the run it checks, as [esp32s3.md](esp32s3.md#the-examples) says.

## ST77916

`packages/engine/tests/circuits/st77916-replay` replays a short start-up and seven writes into an `st77916` panel. `packages/engine/scripts/st77916_example.py` writes the example and its replay list, `events.json`, and the netlist's `spi_replay` events are that list. `--check` fails when either file differs from what the script writes.

`packages/engine/scripts/render_st77916.py EVENTS` plays a replay list into a panel apart from the engine. It reads only the list, never a run's `link.csv` or `display.csv`. Its model is written from the command set [netlist.md](netlist.md#st77916) describes. `--out DIR` writes the rows the run must write, without `t_ps`, and the frame memory it must end with. `--check RUN_DIR` compares a run's `traces/display.csv` row by row on `part`, `x0`, `y0`, `x1`, `y1`, `bytes` and `hash`, and its `traces/display-<part>.rgb565` byte for byte. It also compares `registers`, `flags`, `madctl` and `colmod` with that part's `displays` entry, and checks that the entry names the image file and its SHA-256. It fails when the list gives no row and when the expected image is all zero bytes. `packages/engine/scripts/tests/test_render_st77916.py` checks that the compare passes the right run, and fails a blank frame, a frame missing the last rectangle, a run missing a row, a list with no row, a blank expected image, and a `displays` entry whose `registers`, `flags`, `madctl` or `colmod` differ. `--pattern PATTERN` reads `packages/engine/tests/firmware/esp32s3/pattern.json` in place of a replay list and builds the SPI2 transfers `packages/engine/tests/firmware/esp32s3/pattern.c` sends from it, as [esp32s3.md](esp32s3.md#the-examples) says. It refuses a pattern with a rectangle of one value. With `--check`, each row of the run's `traces/link.csv` must also equal its expected transfer on `part`, `pin`, `kind`, `cs`, `phases`, `bytes`, `hash` and `lanes`, in order. `packages/engine/scripts/tests/test_render_st77916.py` also renders the committed pattern, passes the right run, and fails a run with no row, a run missing a row, a run with an extra row, a blank final image, a rectangle of one value, and a `link.csv` with no row, a missing transfer, an extra one or one on other lanes. It needs Python 3.11 and no package.

`scripts/check.sh full` runs both scripts' checks, runs `orbe-run` on the example, and runs `--check` on that folder.

## Limits

Watches are read at accepted steps. A comparator input that crosses its threshold and crosses back inside one step is missed. The error control keeps steps short where a waveform curves, and `max_step_ps` caps them. `grid_ps` does not.
