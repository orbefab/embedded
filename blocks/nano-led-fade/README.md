# LED fade on a Nano

An LED on D9 of an Arduino Nano fades up and down by PWM. The sketch steps `analogWrite` on D9 from 0 to 255 and back by 5 every 30 ms, so the LED fades up and down about every 3 seconds. D9 is Timer1's OC1A, which runs PWM at about 490 Hz. A 220 ohm resistor sets the LED's current.

Run it here with `orbe check scene.jsonc` and `orbe test`. In this repo a Block reads its parts from the catalog built into `orbe`, so it has no `parts/` and no `parts.lock`.

Install it into a project of your own with `orbe add nano-led-fade`, in an empty folder. That copies the parts into `parts/` and records them in `parts.lock`, as `orbe add <part>` does, then writes `scene.jsonc`, `firmware/` and `tests/`.

`firmware/fade.hex` is built from `firmware/fade/fade.ino` and links the LGPL AVR runtime. [`firmware/NOTICE`](firmware/NOTICE) says where its source is and how to rebuild it.
