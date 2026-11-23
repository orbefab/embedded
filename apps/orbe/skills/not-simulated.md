# orbe: not simulated

Load this when a pin or a feature does nothing, or a wire is `error[not_simulated]`. `orbe part <id>` lists what that part leaves out. `orbe skills get arduino` lists what each board's chip does.

- The Nano's `3V3` holds about 3.3 V from the CH340G while the board has 5 V. `VIN` feeds the AMS1117-5.0, and a wire to `VIN` works.
- The Uno's `VIN` pin and barrel jack are not simulated. The Uno's `3V3` works.
- USB, and serial into the chip. The chip's serial out (TX, D1) is read as text.
- The chip's clamp diodes. A pin pulled past VCC or GND draws no clamp current. `rating.voltage` names it. Load `orbe skills get ratings`.
- A switch's force and travel, and part tolerances.
- orbe never builds firmware. Build it with your toolchain and name the file in the scene. Load `orbe skills get arduino` for a Nano or an Uno.
- `"not_simulated": "<why>"` on a part keeps it for drawing only. Nothing may be wired to it. A wire to one of its pins is `not_simulated`, and the message gives `<why>`.
