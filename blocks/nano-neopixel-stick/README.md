# Rainbow on a NeoPixel stick

The sketch cycles `strip.rainbow(firstHue)` along an 8-pixel stick wired to D6, shifting the rainbow by one pixel (8192 hue units) every 800 ms and calling `show()`. A 470 ohm resistor sits between D6 and DIN.

Rebuild `firmware/rainbow/rainbow.ino` with Adafruit NeoPixel 1.15.5, the library the image was built with. `firmware/NOTICE` names that archive.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
