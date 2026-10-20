# Placeholder models

Each example has `viewer.json` beside `netlist.json`. The file names the 3D model a viewer draws for each part. The engine never reads it. The netlist stays at version 0, and a run folder does not change when this file changes.

`version` is 0. `parts` is one object. Its keys are exactly the part names in that example's netlist. Each value is an object whose `model` is `placeholder:<id>`. `<id>` names the package, in kebab case. Every model in the examples is a placeholder. The real model files live outside this repository. A viewer resolves an id to one of those files. This repository holds only the ids, and an id is never a path into this repository.

| Id | Package |
| --- | --- |
| `capacitor-ceramic` | A ceramic capacitor. Used for every capacitor under 1 uF, including the 22 pF crystal loads, the 10 nF control bypass, and the 100 nF parts. |
| `capacitor-electrolytic` | An electrolytic capacitor. Used for every capacitor of 1 uF or more. |
| `current-source` | A DC current source. |
| `dc-supply` | A DC voltage source. |
| `dip-8` | An NE555 in an 8-pin DIP. |
| `dip-28` | An ATmega328P in a 28-pin DIP. |
| `led-5mm-red` | A 5 mm red LED, the Kingbright WP7113ID the LED examples cite. |
| `potentiometer-rotary` | A rotary potentiometer. |
| `resistor-axial` | An axial resistor. |
| `tactile-switch-6mm` | A 6 mm tactile push button. The switches in the examples are momentary contacts, so they share this id. |

`crates/cli/tests/viewer.rs` scans every `examples/*/netlist.json` and checks the file.
