# Rainbow on a 16-pixel NeoPixel ring

The sketch cycles `ring.rainbow(firstHue)` around a 16-pixel WS2812B NeoPixel ring (`neopixel-ring-16`) wired to D6 through a 470 Ω resistor on `DIN`, shifting the rainbow by one pixel (4096 hue units) every 300 ms and calling `show()`.

Rebuild `firmware/ring/ring.ino` with Adafruit NeoPixel 1.15.5, the library the image was built with. `firmware/NOTICE` names that archive.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
