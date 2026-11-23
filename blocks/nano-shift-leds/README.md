# Eight LEDs on a 74HC595

The sketch calls `shiftOut` with `0x55`, so QA, QC, QE and QG light and the other four stay off. A 220 ohm resistor sets each LED's current. SRCLR is tied to 5V and OE to GND.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
