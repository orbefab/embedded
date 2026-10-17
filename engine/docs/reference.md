# ngspice references

Each example has an operating point from ngspice next to its netlist. A Rust test compares the engine with it. The Rust tests never run ngspice.

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

Stdout is one JSON document. It lists the examples written or checked, the examples skipped with a reason, and with `--check` the files that drifted. A skipped example that already has `ngspice.json` is a failure under `--check`. Its entry's `failure` is `committed ngspice.json was not checked`, and the process exits 1. A skip with no committed reference exits 0. `--self-test` checks those paths without ngspice. Its stdout is one JSON document.

Run it after any change to a `netlist.json`. The Rust test fails until you do.

## deck.cir

The script writes `deck.cir` from `netlist.json`. Do not edit it. `ngspice -b deck.cir` in that directory reruns it and writes `op.raw`.

Parts are written in name order. Names are lowercased, and ground is node `0`. A resistor is `r<name>`, a voltage source is `v<name>`, and a diode or LED is `d<name>` with its own `.model`. Numbers are written so they read back to the same f64.

The options are `temp=25 tnom=25 reltol=1e-9 abstol=1e-15 vntol=1e-12 gmin=1e-12`. 25 C with `tnom` equal to `temp` matches the engine's 298.15 K with no temperature scaling. The tolerances are tighter than ngspice's defaults, so ngspice's own error stays far under the comparison bound.

An example is skipped when a part has no `kind`, when it has a kind the script does not translate, when a part has no `pins` or is missing a pin, when two names are equal ignoring case, when a net other than ground is named `gnd`, which ngspice reads as ground, when a pin names an undeclared net, or when `netlist.json` has a shape the engine's parser would reject. The report lists the skip and the reason in the one JSON document.

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

`op` holds every net voltage and each part's `i` and `v`, under the engine's refs and sign rules. Values come from ngspice's ASCII raw file, which has 16 significant digits. `p` is left out. It is `v * i` on both sides.

## The comparison

`crates/cli/tests/ngspice.rs` runs each example. The run manifest's `input.sha256` must equal `netlist_sha256`. Then every key in `op` must satisfy

```text
|engine - ngspice| <= 1e-3 * |ngspice| + floor
```

The floor is 1e-9 for a voltage and 1e-12 for a current. It only matters near 0. The 1 pA floor does not absorb ngspice's junction gmin, `gmin` times `V`, in cutoff. A reverse-biased example needs a different comparison before it is added.

The engine and ngspice agree to about 3e-7 relative on the LED examples. That gap is consistent with ngspice 42 using older values of `k` and `q` than the exact SI ones. The engine puts its 1e-12 S gmin from every node to ground, and ngspice puts it across each junction. On the divider that difference is about 4e-9 relative.
