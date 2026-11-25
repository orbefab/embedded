# Hello on a 16x2 I2C LCD

The sketch initializes a blue 16 x 2 HD44780 LCD with a PCF8574 I2C backpack at address 0x27 using `LiquidCrystal_I2C`, turns on the backlight, and prints two lines of text (`Hello` and `orbe 16x2 LCD`). The module runs directly at 5 V, lying beside the breadboard with four jumper wires from its right-angle 4-pin header to GND, 5V, A4 (SDA) and A5 (SCL).

`orbe get @lcd` (or `@lcd.text`) prints the two visible 16-character lines, and `orbe get @lcd.backlight` reads the backlight state. The 3D page draws the blue backlit glass with the HD44780 A00 5x8 ROM font, and the run folder saves it as `traces/display-lcd__display.png`.

Rebuild `firmware/hello/hello.ino` with LiquidCrystal_I2C 2.0.0, the library the image was built with. `firmware/NOTICE` names that archive.

Check it with `orbe check scene.jsonc`, lint it with `orbe lint scene.jsonc`, and run its tests with `orbe test`.
