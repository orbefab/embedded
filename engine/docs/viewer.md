# Placeholder models

Each example has `viewer.json` beside `netlist.json`. The file names the 3D model a viewer draws for each part. The engine never reads it. The netlist stays at version 0, and a run folder does not change when this file changes.

`version` is 0. `parts` is one object. Its keys are exactly the part names in that example's netlist. Each value is an object whose `model` is `placeholder:<id>`. `<id>` names the package, in kebab case. Every model in the examples is a placeholder. The real model files live outside this repository. A viewer resolves an id to one of those files. This repository holds only the ids, and an id is never a path into this repository.

| Id | Package |
| --- | --- |
| `capacitor-ceramic` | A ceramic capacitor. Used for every capacitor under 1 uF, including the 22 pF crystal loads, the 10 nF control bypass, and the 100 nF parts. |
| `capacitor-electrolytic` | An electrolytic capacitor. Used for every capacitor of 1 uF or more. |
| `cst816` | A touch controller on an I2C bus, the `cst816` part `examples/atmega-cst816` reads. |
| `current-source` | A DC current source. |
| `dc-supply` | A DC voltage source. |
| `dev-board-328p` | An ATmega328P on a common ATmega328P development board, revision 3, drawn as the whole board. |
| `diode-axial` | An axial leaded rectifier diode, the 1N4001 class the relay example cites. |
| `dip-8` | An NE555 in an 8-pin DIP. |
| `dip-28` | An ATmega328P in a 28-pin DIP. |
| `do-41` | An axial DO-41 diode, the 1N4733A class the zener examples cite. `diode-axial` stays the rectifier. |
| `inductor-radial` | A radial leaded inductor. |
| `led-5mm-red` | A 5 mm red LED, the Kingbright WP7113ID the LED examples cite. |
| `on-board` | A part of the development board model. The board model draws it, so the example does not name a package. |
| `potentiometer-rotary` | A rotary potentiometer. |
| `pulses-replay` | A recorded pin, which `examples/ws2812-replay` replays as timed levels. It has no package of its own. |
| `relay-pcb-5pin` | A 5-pin PCB relay with one changeover contact. |
| `resistor-axial` | An axial resistor. |
| `round-tft-360` | A round 360 by 360 TFT module on a QSPI bus, the `st77916` panel `examples/st77916-replay` draws on. |
| `sot-223` | A SOT-223 package, the LM1117 class the regulator examples cite. |
| `spi-replay` | A recorded SPI host, which `examples/spi-replay` and `examples/st77916-replay` replay. It has no package of its own. |
| `to-92` | A three-lead TO-92 package. Used for every transistor. |
| `ws2812` | An addressable LED of the WS2812E-1313 class, the `ws2812` chain `examples/atmega-ws2812` and `examples/ws2812-replay` drive. |
| `tactile-switch-6mm` | A 6 mm tactile push button. The switches in the examples are momentary contacts, so they share this id. |

`crates/cli/tests/viewer.rs` scans every `examples/*/netlist.json` and checks the file.
