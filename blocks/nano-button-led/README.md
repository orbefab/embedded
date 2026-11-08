# Button and LED on a Nano

A button on D2 lights a red LED on D13 of an Arduino Nano, and releasing it turns the LED off. Pressing the button lights the LED, and releasing it turns it off. A 10k pull-down holds D2 low while the button is open, and a 220 ohm resistor sets the LED's current.

Run it here with `orbe check scene.jsonc` and `orbe test`. In this repo a Block reads its parts from the catalog built into `orbe`, so it has no `parts/` and no `parts.lock`.

Install it into a project of your own with `orbe add nano-button-led`, in an empty folder. That writes `scene.jsonc`, `firmware/` and `tests/`, then copies the parts into `parts/` and records them in `parts.lock`, as `orbe add <part>` does.

`firmware/button.hex` is built from `firmware/button/button.ino` and links the LGPL AVR runtime. [`firmware/NOTICE`](firmware/NOTICE) says where its source is and how to rebuild it.
