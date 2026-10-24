# Versions

Three numbers version what the engine reads and writes. This page says what each covers and which changes move it.

| Number | Where | Covers |
| --- | --- | --- |
| netlist `version` | `netlist.json`, and `netlist_format` in the manifest | which documents are valid, and what each one means |
| `run_format` | `manifest.json` | every file in the run folder, `traces/` included |
| `engine_version` | `manifest.json` | how the engine computes a run, described in [engine.md](engine.md) |

## Netlist version

A version names a set of valid documents and what each one means. A change that keeps every valid document valid, with the same meaning, keeps the version. These keep it:

- a new part kind
- a new source form or a new waveform kind
- a new analysis
- a new value of a string enum, such as a `basis`
- a rule that accepts more documents than before

Every other change bumps it. A new field on an existing object bumps it, because every field is required and an unknown field is an error.

The engine reads the versions in a list. This engine reads `[0]`. `netlist.version`'s `data.supported` is that list, and `sim doctor` reports it as `netlist_versions`. A document whose `version` is not in the list yields only `netlist.version`. The engine reads an older listed version with that version's own rules, and gives it its old meaning.

## Run format

`run_format` versions the whole folder. A reader that knows one `run_format` can read every folder written with it.

- A reader ignores a key it does not know, in every JSON object, and a file it does not know.
- A new key, a new file, a new diagnostic code and a new severity keep `run_format`. `traces/envelope.f64` and the `envelope` key in `traces/index.json` are such additions, and the index `format` stays 0.
- Removing or renaming a key, or changing a type, a unit, a meaning or a binary layout, bumps it.

`traces/index.json` keeps its own `format`. A change that bumps it also bumps `run_format`, so `run_format` alone answers whether a reader can read the folder.

`sim run` replaces a previous run of any `run_format`. [run-folder.md](run-folder.md#replacing-a-directory) says how it recognises one.

## Diagnostics

These are stable within a `run_format`:

- `code`, and the condition the code names. A condition never moves to another code.
- the shape of `refs`
- what `t_ps` means
- the keys of `data` and their types

`message` is for people. Its text can change in any version, so a reader matches on `code` and `data`, never on `message`. New codes, new severities and new `data` keys can appear without a bump, and readers accept them.

## Engine version

`engine_version` covers everything [engine.md](engine.md) describes: the step policy, how chip events are found, and the accuracy the engine reaches. A change there can move trace values within the documented accuracy without changing either format.

This engine is `0.12.0`. A part can be an `nmos` MOSFET, as [netlist.md](netlist.md#n-channel-mosfet) says. A new kind keeps the netlist version at 0. Its ref `#<part>.id` is a new value, so `run_format` stays 0. No new file, key, or code. `examples/nmos-led` is new. No other example's run folder moves apart from `engine_version`, because no existing part's stamp changes.

Engine `0.11.0` is the version in which a part can be an `npn` or a `pnp` transistor, as [netlist.md](netlist.md#npn-and-pnp-transistors) says. A new kind keeps the netlist version at 0. Its refs `#<part>.ib` and `#<part>.ic` are new values, so `run_format` stays 0. No new file, key, or code. `examples/npn-led`, `examples/pnp-load` and `examples/ce-amp` are new. No other example's run folder moves apart from `engine_version`, because no existing part's stamp changes.

Engine `0.10.0` is the version in which a voltage or current source can hold a `serial` waveform, as [netlist.md](netlist.md#source-waveforms) says. The netlist version stays 0 and `run_format` stays 0. No new file, key, or code. No example run folder moves apart from `engine_version`.

Engine `0.9.0` computes `exp`, `ln` and `cbrt` with the `libm` crate, so a run folder does not depend on the platform's math library, as [engine.md](engine.md) says. Under `0.8.0` they came from the platform's math library, and a `wasm32` run folder differed from a native one in the last bits of its analog traces. Every example's run folder is now the same on x86-64 Linux and on `wasm32-unknown-unknown`. On x86-64 Linux, `traces/analog.f64` and `traces/envelope.f64` of `examples/atmega-blink`, `examples/atmega-button` and `examples/atmega-fade` move by at most 6 units in the last place, a relative 1e-15. Those of `examples/ne555-led` move by at most 61, a relative 1.1e-14. No logic edge and no observation moves, and no other file of any example moves apart from the version. The netlist version stays 0 and `run_format` stays 0.

Engine `0.8.0` is the version in which a transient run of a netlist with a USART writes `traces/serial.csv`, one row per byte each USART sent, as [run-folder.md](run-folder.md#traces) says. A new file keeps `run_format` at 0, and `traces/index.json` gains no key. `examples/atmega-analog-serial`, `examples/atmega-blink`, `examples/atmega-button` and `examples/atmega-fade` gain the file. No other file of any example moves apart from the version. The netlist version stays 0.

Engine `0.7.0` is the version in which a number literal parses to the correctly rounded `f64`. An export reprints those numbers. A batch run parses the reprint, so the literal has to mean the same value both times. A netlist whose literal the old parser read 1 ulp off now computes from the value it states, so its results can move in the last place: with a saturation current of `1e-50`, `#d1.i` moves from 2.023918337867733 to 2.023918337867734. No example run folder moves apart from the version. The version also reads `steps`. The netlist version stays 0 and `run_format` stays 0.

Engine `0.6.0` is the version in which an edge whose restart solve moves no capacitor keeps the segment and the step it wanted, as [engine.md](engine.md#transient-steps) says. Each capacitor sums what the kept edges could move it over the longest step, and a segment starts once a sum passes 1 nV. The point before the edge keeps its pre-edge slope, because each capacitor's change in current `di` is added to the current stored there. So the next step's error estimate reads no kink of `di` over the step that landed on the edge. Edges that feed a capacitor still start a segment at 1 ns. A stiffness try now holds until an accepted step at least as long as the one it waited for. A try that a watch, a breakpoint or a core stop cuts, or that the step control shrinks, tries again, so a run with stiff capacitors can take other steps with no edge at all. No example does. `examples/rc-switch`, `examples/v-pulse-rc`, `examples/current-pulse` and `examples/ne555-led` keep their bytes, and so does `examples/atmega-analog-serial` at every grid and step. `examples/atmega-blink`, `examples/atmega-button`, `examples/atmega-fade` and `examples/button-led` move at their committed settings by at most 1.8e-15 V, a few units in the last place, and `examples/atmega-fade` moves by the same at a 10 ms step with no grid. No logic edge and no observation moves. `examples/atmega-analog-serial` stopped at 16 s with that step and no grid accepts 83252 steps where `0.5.0` accepted 1424993. The netlist version stays 0 and `run_format` stays 0.

Engine `0.5.0` made a trace row a probe and never a landing, as [engine.md](engine.md#rows) says. The grid no longer moves the steps, so every example's analog values moved once, and so did some located chip events. `examples/v-pulse-rc` moved by up to 7.9e-5 of a voltage's span, and the OUT edges of `examples/ne555-led` by up to 5.3e-7 of their time. Both are within the accuracy [engine.md](engine.md#accuracy) gives. The netlist version stays 0 and `run_format` stays 0. A row whose probe fails twice is the line between the steps around it, with the new warning `trace.row_interpolated`. A new code keeps `run_format` at 0.

Engine `0.4.0` added the `atmega328p` kind. A new part kind keeps the netlist version at 0. Its new diagnostic codes, `netlist.firmware` and `netlist.same_net`, keep `run_format` at 0. So do its logic signals, which are new refs in `observations.logic` and in the `logic` key of `traces/index.json`. A netlist without the kind runs as it did under `0.3.0`.
