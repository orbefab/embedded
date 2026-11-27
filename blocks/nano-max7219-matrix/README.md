# Smiley on a MAX7219 8 x 8 LED matrix

The sketch drives a single MAX7219 8 x 8 LED dot matrix module with LedControl on D11 (DIN), D13 (CLK) and D10 (CS), wakes the driver from power-up shutdown, sets intensity 8, and draws an 8 x 8 smiley face.

`tests/matrix.screen` holds the 8 x 8 pattern, `#` lit and `.` dark. `orbe get @matrix` prints the same rows and per-dot current, and `orbe get @matrix --json` includes per-dot brightness in amperes.

Rebuild `firmware/matrix/matrix.ino` with LedControl 1.0.6. `firmware/NOTICE` names the pinned archives.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
