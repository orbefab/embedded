# Diagnostics

A diagnostic is one JSON object. The fields are written in this order.

| Field | Meaning |
| --- | --- |
| `code` | Stable id, `word.word` with lowercase letters and `_` in the tail. |
| `severity` | `error` or `warning`. |
| `refs` | Refs into the netlist. Empty when the code names no net, part, or pin. |
| `t_ps` | Picoseconds from t = 0, or `null` when the finding is about the input. |
| `message` | One sentence for a person. |
| `data` | Extra fields. Keys are alphabetical. |

The run sorts diagnostics by `t_ps`, then `code`, then `refs`, then the JSON bytes of the whole object. A `null` time sorts before a number, so every input finding precedes a solver finding.

A run whose diagnostics are all warnings still writes the operating point. A run with any error writes `{}` for `observations.json`.

Stdout of `sim run` lists the same objects in the same order.

## netlist.syntax

Severity `error`. No refs. `t_ps` is `null`.

The message is:

```text
schema mismatch at `{path}` ({detail})
```

`data` holds `column`, `detail`, `line`, and `path`. `detail` is the parser text. A typo such as `resistence` lands here. A repeated pin key lands here, with that key in `detail`. Bytes after the JSON value land here. Trailing whitespace does not. Any other `version` is reported only as `netlist.version`, including when the rest of the document is broken.

## netlist.version

Severity `error`. No refs. `t_ps` is `null`.

The message is:

```text
netlist version {found} is not supported; this engine reads {supported}
```

`data` holds `found` and `supported`. `supported` is 0.

## netlist.name

Severity `error`. No refs. `t_ps` is `null`.

The message is:

```text
name `{name}` at `{path}` is not valid ({reason})
```

`data` holds `name`, `path`, and `problem`. `problem` is `empty`, `too_long`, `bad_char`, or `reserved`.

`reserved` is a part named `net`. The name grammar is in [netlist.md](netlist.md).

## netlist.duplicate

Severity `error`. The `refs` array holds one ref, `#<part>` or `#net.<name>`. `t_ps` is `null`.

The message is `{ref} is declared more than once`.

`data` is `{}`.

## netlist.unknown_net

Severity `error`. Refs is the pin, for example `#r2.pin.b`. `t_ps` is `null`.

The message is:

```text
pin #r2.pin.b names net `gnd2`, which is not in `nets`
```

`data` holds `net`, the missing name.

## netlist.pins

Severity `error`. Refs is the part. `t_ps` is `null`.

The message is `{ref} pins do not match its kind`.

`data` holds `missing` and `unexpected`. `missing` follows the kind's pin order. `unexpected` is sorted by name.

## netlist.unit

Severity `error`. Refs is the part. `t_ps` is `null`.

The message is:

```text
{ref} {field} is in `{found}`; it must be in `{expected}` (SI base units)
```

`data` holds `expected`, `field`, and `found`. Resistance must be `ohm`. Voltage must be `V`.

## netlist.value

Severity `error`. Refs is the part. `t_ps` is `null`.

The message for `positive` is `{ref} {field} is {value}; it must be positive`.

The message for `finite_reciprocal` is `{ref} {field} is {value}; its reciprocal must be finite`.

`data` holds `field`, `rule`, and `value`. `rule` is `positive` or `finite_reciprocal`. Resistance must be positive, and its reciprocal must be finite. A value that breaks both reports `positive`. Voltage only has to be finite and in `V`. A number outside the f64 range is `netlist.syntax`.

## netlist.ground

Severity `error`. No refs. `t_ps` is `null`.

The message is:

```text
ground `{net}` is not in `nets`
```

`data` holds `net`.

## net.floating

Severity `warning`. Refs is `#net.<name>`. `t_ps` is `null`.

The message is `{ref} has no DC path to ground`.

`data` is `{}`.

A DC path is a chain of resistors and voltage sources from the net to ground. Each of those parts joins its two pins. gmin, 1e-12 S from every node to ground, is separate from that chain.

A resistor from `vin` to `lonely`, with a voltage source from `vin` to `gnd`, puts `lonely` on a path to ground. A net that no part names is floating. Two nets whose only part is a resistor between them are floating. The operating point is still written, and `sim run` exits 0.

## solver.singular

Severity `error`. Refs is the unknown the zero pivot names, a net or a part. `t_ps` is 0 for the operating point.

The message is `singular matrix at {ref}`.

`data` is `{}`. `observations.json` is `{}`.

Two voltage sources in parallel, set to different voltages, are singular. With `v1` at 5 V and `v2` at 3 V, both from `vin` to `gnd`, the ref is `#v2`.

## solver.nonfinite

Severity `error`. Refs is the unknown or the part. `t_ps` is 0 for the operating point.

The message is `non-finite number at {ref}`.

`data` is `{}`. `observations.json` is `{}`.

A solution component that is not finite names that unknown. An observed `i`, `v`, or `p` that is not finite names the part. A 1e200 V source across 1 ohm overflows `p` on the resistor.

## solver.too_large

Severity `error`. No refs. `t_ps` is 0 for the operating point.

The message is `{unknowns} unknowns exceeds the dense solver limit of {limit}`.

`data` holds `limit` and `unknowns`. `limit` is 4096. The solver checks the count before it allocates the dense matrix. `observations.json` is `{}`.
