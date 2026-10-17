# Netlist v0

A netlist is fully resolved JSON. `version` is 0. Every field is required, and an unknown field is an error. Nothing is filled in by default.

Values are SI numbers. `unit` must already be the field's unit.

## Document

`examples/divider/netlist.json` is a 5 V source across `vin` and `gnd`, 1000 ohm from `vin` to `vout`, and 2000 ohm from `vout` to `gnd`.

```json
{
  "version": 0,
  "seed": 0,
  "analysis": { "op": {} },
  "ground": "gnd",
  "nets": ["gnd", "vin", "vout"],
  "parts": [
    {
      "name": "v1",
      "kind": "voltage_source",
      "pins": { "p": "vin", "n": "gnd" },
      "voltage": { "value": 5.0, "unit": "V", "source": "user" }
    },
    {
      "name": "r1",
      "kind": "resistor",
      "pins": { "a": "vin", "b": "vout" },
      "resistance": {
        "value": 1000.0,
        "unit": "ohm",
        "source": {
          "datasheet": {
            "doc": "resistor-nominal",
            "at": "Table 1, nominal",
            "typical": false
          }
        }
      },
      "ratings": { "power": null }
    },
    {
      "name": "r2",
      "kind": "resistor",
      "pins": { "a": "vout", "b": "gnd" },
      "resistance": { "value": 2000.0, "unit": "ohm", "source": "user" },
      "ratings": { "power": null }
    }
  ]
}
```

## Fields

`version` is `0`. Any other number yields only `netlist.version`, even when the rest of the document is valid.

`seed` is an unsigned integer. The manifest records it. Changing `seed` leaves the operating point unchanged.

`analysis` is `{"op": {}}`. The object must be empty. A string such as `"op"` is `netlist.syntax`.

`ground` names one entry of `nets`. That net is held at 0 V. A name absent from `nets` is `netlist.ground`.

`nets` lists every net name. `parts` lists every part.

The engine stores ground first and sorts the other nets by name. It sorts parts by name. Reordering `nets` or `parts` in the JSON does not change observations or diagnostics.

## Names

A name is an ASCII letter, then ASCII letters, digits, or `_`, from 1 to 64 bytes.

A part named `net` is `netlist.name` with problem `reserved`. Pin, unit, and value checks on that part are still reported. A name that fails the grammar is `netlist.name` and those checks are not run, because the part has no ref. A net may be named `net`.

Two nets, or two parts, with the same name are `netlist.duplicate`. The ref is `#net.<name>` or `#<part>`. Pin, unit, and value checks on a duplicate part are still reported.

## Parts

`kind` selects the part. Pin keys must be exactly that kind's pin names. A missing or extra key is `netlist.pins`. An extra key that is a legal name and names a net absent from `nets` is also `netlist.unknown_net`.

### Resistor

`kind` is `resistor`. Pins are `a` and `b`, in that order for the sign rule.

`resistance.value` is in `ohm`, greater than 0, and its reciprocal must be finite. Zero or a negative value is `netlist.value` with rule `positive`. A positive value whose reciprocal overflows is `netlist.value` with rule `finite_reciprocal`. A different unit is `netlist.unit`. A number outside the f64 range is `netlist.syntax`.

`ratings` holds `power`. See [Ratings](#ratings).

### Voltage source

`kind` is `voltage_source`. Pins are `p` and `n`. The source sets `V(p) - V(n)` equal to `voltage.value`.

`voltage.value` is in `V` and may be any finite number, including 0 and negative values. A different unit is `netlist.unit`.

A voltage source has no `ratings` field.

### Diode and LED

`kind` is `diode` or `led`. Both take the same fields and solve the same equation. Pins are `anode` and `cathode`.

The part is a junction in series with a resistance. The junction current is `Is * (exp(vj / (N * kT/q)) - 1)`. `V(anode) - V(cathode)` is `vj` plus the current times `Rs`.

| Field | Symbol | Unit |
| --- | --- | --- |
| `saturation_current` | `Is` | `A` |
| `emission_coefficient` | `N` | `1` |
| `series_resistance` | `Rs` | `ohm` |

Each value must be greater than 0, and its reciprocal must be finite. Those are the resistor's rules. `n*VT` must also be a normal number, and `vcrit` must be finite and positive. `VT` is `k*T/q` at 298.15 K with the exact SI values of `k` and `q`. `vcrit` is `n*VT*ln(n*VT/(sqrt(2)*Is))`, which is positive when `n*VT > sqrt(2)*Is`. A junction that breaks either condition is `netlist.value` with rule `finite_junction`. The field is `emission_coefficient` when `n*VT` is not normal. It is also `emission_coefficient` when `vcrit` is not finite and positive and `Is` is a normal number. The field is `saturation_current` when `vcrit` is not finite and positive and `Is` is not a normal number.

The device is at 25 C, 298.15 K. `k` and `q` are the exact SI values. There is no temperature field and no temperature scaling. A value fitted at another temperature does not describe the part here.

`ratings` holds `current` and `power`.

`examples/led-5v` and `examples/led-9v` use a Kingbright WP7113ID, a red 5 mm LED. Its `Is`, `N`, and `Rs` solve the equation exactly through three points read off the datasheet's forward current figure at 25 C: (1.70 V, 2 mA), (1.90 V, 10 mA), and (2.00 V, 20 mA). The fit gives `Is` = 6.711e-10 A, `N` = 4.427, and `Rs` = 2.115 ohm. Each value's `at` names the figure, the points, and the temperature, so the fit can be redone from the citation.

## Ratings

A rating bounds the stress on a part. Every key of a kind's `ratings` object is required. Each value is a value object, or `null` for a part that is deliberately unrated.

| Kind | Keys | Checked quantity |
| --- | --- | --- |
| `resistor` | `power` | `#<part>.p` |
| `diode`, `led` | `current`, `power` | `#<part>.i`, `#<part>.p` |

`current` is in `A`. `power` is in `W`. A rating must be greater than 0. A missing key, an unknown key, and a missing `ratings` object are `netlist.syntax`. A wrong unit is `netlist.unit` with field `ratings.current` or `ratings.power`. A value of 0 or below is `netlist.value` with the same field.

```json
"ratings": { "power": { "value": 0.25, "unit": "W", "source": "user" } }
```

The run compares the magnitude of each rated quantity with its rating. Going over is a warning, `rating.current` or `rating.power`, in [diagnostics.md](diagnostics.md).

## Sources

`source` is one of three forms.

- `"user"`
- `{"closed_form": {"note": "<text>"}}`
- `{"datasheet": {"doc": "<id>", "at": "<place>", "typical": false}}`

`note`, `doc`, and `at` are strings. `typical` is a boolean. The solve uses `value` alone.

## Sign rules

For every two-terminal part, pin 0 is the first pin listed above.

`i` is the current entering pin 0.

`v` is the voltage of pin 0 minus the voltage of pin 1.

`p` is `v * i`, the power the part absorbs. A source that delivers power has negative `i` and negative `p`.

For a diode or LED, `i` is the current through `Rs`, and `p` includes the power in `Rs`. The junction's internal node is not observed.

On the divider, `#r1.i` and `#r2.i` are positive, `#r1.p` and `#r2.p` are positive, and `#v1.i` and `#v1.p` are negative.

## Refs

A ref names one net, part, pin, or quantity.

```text
ref      = "#" ( "net." name | name [ "." ( quantity | "pin." name ) ] )
quantity = "i" | "v" | "p"
```

| Text | Meaning |
| --- | --- |
| `#net.vout` | Net `vout` |
| `#r1` | Part `r1` |
| `#r1.pin.a` | Pin `a` of `r1` |
| `#r1.pin.p` | Pin `p` of `r1`. Power of `r1` is `#r1.p`. |
| `#r1.i` | Current of `r1` |
| `#r1.v` | Voltage of `r1` |
| `#r1.p` | Power of `r1` |

These texts parse and format back to the same text. The following texts are rejected.

- `r1`, because the `#` is missing
- `#`, `#net.`, `#r1.pin`, `#r1.pin.`, `#r1.`
- `#net.1vout`, `#1r`, because the name is illegal
- `#r1.x`, `#r1.i.v`

`#net.vout` is a net. `#net` is a part named `net`.

Map keys and diagnostic order use this order: nets, then parts, then pins, then quantities. Quantities sort `i`, then `v`, then `p`. Names sort by byte order inside one group.
