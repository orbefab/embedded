# Rainbow on a NeoPixel stick

The sketch calls `strip.rainbow()` on an 8-pixel stick wired to D6, then `show()`. A 470 ohm resistor sits between D6 and DIN. Each pixel keeps its own colour.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
