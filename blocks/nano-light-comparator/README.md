# Light comparator on a Nano

An LDR and a trimmer set a threshold on an LM358. In the dark the output goes high, a red LED lights, and the Nano prints `dark`. In the light the LED is off and the Nano prints `light`. The sketch prints once at the start and again only when D2 changes.

The LDR sits above 2.2k to ground, so the node rises as the room gets brighter. It is 2.65 V at the scene's 300 lx. A 10k trimmer, fed from 5 V through 4.7k and set at `position` 0.5, holds the other input near 1.62 V while the output is low. A 68k resistor from the output back to that input raises the threshold while the LED is on, so the light has to climb further before the LED goes out, and the LED does not flicker on the way through. Both inputs stay under 3.5 V through 1000 lx. Brighter than that, this divider climbs past the LM358's input range. Change `position` in the scene to move the threshold. Nothing turns the trimmer during a run.

`orbe set light 10` darkens the room and the LED comes on. `orbe set light 300` brings the room back and the LED goes out.

Run it here with `orbe check scene.jsonc` and `orbe test`. In this repo a Block reads its parts from the catalog built into `orbe`, so it has no `parts/` and no `parts.lock`.

Install it into a project of your own with `orbe add nano-light-comparator`, in an empty folder. That copies the parts into `parts/` and records them in `parts.lock`, as `orbe add <part>` does, then writes `scene.jsonc`, `firmware/` and `tests/`.

`firmware/light.hex` is built from `firmware/light/light.ino` and links the LGPL AVR runtime. [`firmware/NOTICE`](firmware/NOTICE) says where its source is and how to rebuild it.
