# Button and LED on an Uno

Pressing the button lights the LED and the Uno's L LED, which also sits on D13. Releasing it turns both off. A 10k pull-down holds D2 low while the button is open, and a 220 ohm resistor sets the LED's current.

`firmware/button.hex` is built from `firmware/button/button.ino` and links the LGPL AVR runtime. [`firmware/NOTICE`](firmware/NOTICE) says where its source is and how to rebuild it.
