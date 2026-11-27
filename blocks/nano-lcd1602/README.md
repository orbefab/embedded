# Hello on a 16x2 parallel LCD

The sketch is the `HelloWorld` example of the Arduino `LiquidCrystal` library, unchanged. It drives a blue 16 x 2 HD44780 LCD over a 4-bit bus: RS on D12, E on D11 and D4 to D7 on D5 to D2, with R/W tied to ground. It prints `hello, world!` on the top line and the seconds since reset on the bottom line. A 10 kohm trimmer across 5V and ground sets V0 near 0.9 V, which gives the glass its contrast, and 220 ohm from 5V lights the backlight at about 5 mA.

`orbe get @lcd.text` prints the two visible 16-character lines, `orbe get @lcd.display.screen` draws the glass with a `#` for each lit dot, and `orbe get @lcd.backlight` reads the backlight. Turn the trimmer up toward 5V with `orbe set @trim.position 0.9` and the glass goes blank, and a `chip.protocol` warning names the contrast. The 3D page draws the backlit glass with the HD44780 A00 5x8 ROM font.

The run is quiet. The `LiquidCrystal` constructor runs a `begin` of its own as soon as the sketch starts, and the `begin` in `setup()` starts the controller again. The Nano starts the sketch about 66 ms after power-on: its fuses hold the chip in reset for 16K CK, 14 CK and 65 ms, and Optiboot then goes straight to the sketch. By then the HD44780U's 15 ms power-on wait is over, as on the bench.

Rebuild `firmware/hello/hello.ino` with LiquidCrystal 1.0.7, the library the image was built with. `firmware/NOTICE` names that archive.

Check it with `orbe check scene.jsonc`, lint it with `orbe lint scene.jsonc`, and run its tests with `orbe test`.
