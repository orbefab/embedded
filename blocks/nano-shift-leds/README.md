# Eight LEDs on a 74HC595

The sketch calls `shiftOut` to walk a single lit LED from QA to QH and back, stepping every 100 ms. A 220 ohm resistor sets each LED's current. SRCLR is tied to 5V and OE to GND.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
