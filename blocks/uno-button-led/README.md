# Button and LED on an Uno

A button on D2 lights a red LED on D13 of an Arduino Uno, and the board's own L LED with it. Pressing the button lights the LED and the Uno's L LED, which also sits on D13. Releasing it turns both off. A 10k pull-down holds D2 low while the button is open, and a 220 ohm resistor sets the LED's current.

Run it here with `orbe check scene.jsonc` and `orbe test`. In this repo a Block reads its parts from the catalog built into `orbe`, so it has no `parts/` and no `parts.lock`.

Install it into a project of your own with `orbe add uno-button-led`, in an empty folder. That copies the parts into `parts/` and records them in `parts.lock`, as `orbe add <part>` does, then writes `scene.jsonc`, `firmware/` and `tests/`.

`firmware/button.hex` is built from `firmware/button/button.ino` and links the LGPL AVR runtime. [`firmware/NOTICE`](firmware/NOTICE) says where its source is and how to rebuild it.
