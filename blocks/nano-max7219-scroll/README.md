# Scrolling text on a 4-in-1 MAX7219 LED matrix

The sketch scrolls `"ORBE"` from right to left across a 4-in-1 (32 x 8) FC-16 MAX7219 LED matrix module over hardware SPI (D11 MOSI to DIN, D13 SCK to CLK, D10 to CS) using MD_Parola and MD_MAX72XX (`FC16_HW`).

`orbe get @matrix` stitches all four 8 x 8 tiles left to right into one 32 x 8 bitmap with per-dot brightness. `tests/scroll.test` checks two frames 200 ms apart to verify the text scrolls across the chained modules.

Rebuild `firmware/scroll/scroll.ino` with MD_Parola 3.7.7 and MD_MAX72XX 3.5.1. `firmware/NOTICE` names the pinned archives.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
