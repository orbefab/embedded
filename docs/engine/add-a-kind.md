# Add an analog kind

An analog kind is a part the engine solves as conductances, sources and junctions, such as `resistor`, `zener` or `nmos`. A new one is one new file, one `mod` line and one row in `packages/engine/src/netlist/catalog.rs`. The row generates the `Model` variant, the wire variant that reads the part's JSON, the call to the kind's resolve function, and the kind's entry in `engine_kinds`, the list of names and pins that `orbe check` reads. Two kind PRs touch the same file only at their rows.

Copy the existing kind closest to yours. `packages/engine/src/passive/resistor.rs` is the smallest.

## Write the kind file

Create `packages/engine/src/<group>/<kind>.rs`. `<group>` is `passive` for a linear part or a source, `semiconductor` for a junction device, or `chip` for a behavioural model of a packaged part, such as the linear regulator or the op-amp. The file holds these, in this order:

1. The part struct. It has `pub pins: [NetId; N]` and one field per param, each a `Value` in SI units with its rule in a doc comment. `impl <Kind> { pub const PINS: [&'static str; N] }` names the pins in the order of `pins`.
2. The wire struct, `Wire<Kind>`, which serde reads from the netlist. Derive `Deserialize` with `#[serde(deny_unknown_fields)]`. It has `pub(crate) name: String`, `pins: PinMap`, and one `WireValue` per param.
3. The resolve function, `pub(crate) fn resolve_<kind>(ctx: ParseCtx<'_>, wire: Wire<Kind>) -> Option<Kind>`. Check each field with the helpers in `packages/engine/src/netlist/json.rs`, such as `resolve_pins`, `resolve_value` and `resolve_rating`. Each helper pushes its own diagnostic. Return `None` when any field is broken.
4. `impl Kind for <Kind>`. The `Kind` trait in `catalog.rs` documents each method. Put the stamp math in `stamp`.

Mark `stamp` `#[inline]`. It runs on every Newton iteration, and the release build splits the crate into several code units, so without the attribute the call can stay a call.

Add `pub(crate) mod <kind>;` and `pub use <kind>::<Kind>;` to the group's module file: `passive/mod.rs`, `semiconductor/mod.rs` or `chip.rs`.

## Add the row

Add one row at the end of the `analog` group of `define_kinds!` in `catalog.rs`. The resistor's row is:

```rust
Resistor(crate::passive::Resistor), "resistor",
	crate::passive::resistor::WireResistor, crate::passive::resistor::resolve_resistor,
	&crate::passive::Resistor::PINS;
```

The columns are the `Model` variant with the part type, the kind name, the wire type, the resolve function and the pin names. The kind name is both the netlist's `"kind"` tag and the name `orbe check` accepts, so the two cannot differ. A `#[doc = "..."]` line above the row documents the variant.

Serde's error for an unknown kind lists every kind in row order, so the new kind joins that list where its row sits.

## Let the compiler find what is missing

Run `cargo check -p orbe-engine --all-targets`. It reports each missing piece at the place to fix it.

- A row with no `impl Kind` is one `E0277` at the row: the trait `Kind` is not implemented for the part type.
- An impl that leaves out a required method is one `E0046` at the impl. It names each missing method with its signature. The schedule methods, `next_change_after`, `level`, `level_mut`, `serial` and `serial_mut`, default to none.
- A resolve function that takes another wire type, or returns another part type, is one `E0308`. It prints the signature the row expects and the one it found.

## Check what the compiler cannot

The compiler makes each kind answer every method. It cannot tell whether the answer is right.

- `ratings` names the quantity a current rating bounds. Pick the current the datasheet rates, such as `ic` on a BJT.
- `conduction` says how the part carries a steady current. `orbe check` follows it to find an LED with nothing in series to limit its current.
- `dc_paths` lists the pin pairs that conduct at DC. A pair left out makes `net.floating` report a net that is driven.
- `quantities` and `observe` must agree in order and count.
- A setting from a session goes through `level` or `serial`. A switch's contact is the one setting with its own arm, in `packages/engine/src/session/input.rs`.

## Add the tests

- Add `packages/engine/tests/netlist/<kind>.rs` and its `mod` line in `main.rs`. Parse a part with every field and check the fields, `pin_names`, `quantities` and `ratings`. Break each field and check its diagnostic.
- Add an example under `packages/engine/tests/circuits/<name>/` that runs the kind. When ngspice has a model for it, add the kind's deck line and pins to `packages/engine/scripts/ngspice_refs.py` and commit `deck.cir` and `ngspice.json`. [reference.md](reference.md) says how. When it does not, the script skips the example on its own. Say why under that page's skips, and add a branch for the kind to `matches_its_reference` in `packages/engine/tests/circuit/runs/ngspice.rs`, which otherwise fails on an example with no `ngspice.json`. Add the example's name to that file's list of examples too.
- The catalog's tests need no edit. They check that every row has a unique name and unique pins, and that `orbe check` lists every wire kind except `esp32s3`.

## Write the docs

- Add a section for the kind under `## Parts` in [netlist.md](netlist.md): its pins, each field with its unit and rule, its quantities, and its ratings.
- Add the kind to the list of what the engine solves at the top of [README.md](README.md).

## Show the result in the PR

- `scripts/run-hashes.sh` on main and on the branch: every existing line is unchanged, and the new example adds its own lines.
- The example's ngspice reference passes, or the PR says why the kind has none.
- `scripts/check.sh full` passes.
- When the kind changes a stamp that runs often, `scripts/check-speed.sh` medians on main and on the branch.

## A chip or a device

A chip kind, such as `ne555`, or a device on the `orbe-model` contract, is a row in the `chip` group of the same macro. Its resolve function returns the chip struct, and the row puts it in a `Chip` variant. Add that variant to `Chip` in `packages/engine/src/netlist.rs`. The compiler then names each `match` on `Chip` that needs an arm. [model.md](model.md) says what a device implements.
