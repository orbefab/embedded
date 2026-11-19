# Light comparator on a Nano

An LDR and a trimmer set a threshold on an LM358. In the dark the output goes high, a red LED lights, and the Nano prints `dark` and a count. In the light the LED is off and the Nano prints `light` and a count. The sketch prints once at the start and again only when D2 changes. A run that goes dark and comes back prints `light 1`, then `dark 2`, then `light 3`.

The LDR sits above 2.2k to ground, so the node rises as the room gets brighter. It is 2.52 V at the scene's 300 lx. A 10k trimmer, fed from the 5V pin through 4.7k and set at `position` 0.5, holds the other input near 1.54 V while the output is low. A 68k resistor from the output back to that input raises the threshold while the LED is on, so the light has to climb further before the LED goes out, and the LED does not flicker on the way through. A higher `position` moves the wiper toward pin b, at ground, and the threshold falls. At 0.25 the LED comes on near 220 lx and goes out near 280 lx. At 0.75 it comes on near 16 lx and goes out near 21 lx. Nothing turns the trimmer during a run. Both inputs stay inside the LM358's rated input range, up to 1.5 V under its supply, through about 890 lx. Brighter than that, the LDR divider climbs past it. The trimmer side stays inside that range, the output stays low, and the Block still prints `light`.

On USB the Nano's 5V pin is 4.76 V, one Schottky drop under the host's 5.0 V, and every voltage above scales with it. On a typical USB port a real board reads about 4.55 V to 4.8 V there, so its voltages differ by a few percent, while the light levels where the LED switches stay where they are.

`orbe set light 10` darkens the room and the LED comes on. `orbe set light 300` brings the room back and the LED goes out.

`firmware/light.hex` is built from `firmware/light/light.ino` and links the LGPL AVR runtime. [`firmware/NOTICE`](firmware/NOTICE) says where its source is and how to rebuild it.
