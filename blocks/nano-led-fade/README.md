# LED fade on a Nano

The sketch steps `analogWrite` on D9 from 0 to 255 and back by 5 every 30 ms, so the LED fades up and down about every 3 seconds. D9 is Timer1's OC1A, which runs PWM at about 490 Hz. A 220 ohm resistor sets the LED's current.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.

`firmware/fade.hex` is built from `firmware/fade/fade.ino` and links the LGPL AVR runtime. [`firmware/NOTICE`](firmware/NOTICE) says where its source is and how to rebuild it.
