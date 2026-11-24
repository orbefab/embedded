# Hello on a 0.96 inch OLED

The sketch starts a 128 x 64 SSD1306 OLED module at address 0x3C with Adafruit SSD1306, then draws a frame, the word Hello and a filled circle with Adafruit GFX, and calls `display()`. The module's header plugs into the breadboard, and its SCL and SDA run to A5 and A4.

`tests/hello.screen` holds what the glass shows, one line a row, `#` lit and `.` dark. `orbe get @oled.display.screen` prints the same rows, and the run folder keeps them as `traces/display-oled__display.png`.

Rebuild `firmware/hello/hello.ino` with Adafruit SSD1306 2.5.17 and Adafruit GFX 1.13.0, the libraries the image was built with. `firmware/NOTICE` names those archives.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
