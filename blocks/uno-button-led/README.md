# Button and LED on an Uno

A 10k pull-down holds D2 low while the button is open, and a 220 ohm resistor sets the LED's current.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.

`firmware/button.hex` is built from `firmware/button/button.ino` and links the LGPL AVR runtime. [`firmware/NOTICE`](firmware/NOTICE) says where its source is and how to rebuild it.
