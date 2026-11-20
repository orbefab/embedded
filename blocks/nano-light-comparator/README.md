# Light comparator on a Nano

An LDR and a trimmer set a threshold on an LM358. In the dark the output goes high, a red LED lights, and the Nano prints `dark` and a count. In the light the LED is off and the Nano prints `light` and a count. The sketch prints once at the start and again only when D2 changes. A run that goes dark and comes back prints `light 1`, then `dark 2`, then `light 3`.

The LDR sits above 2.2k to ground, so the node rises as the room gets brighter. It is 2.52 V at the scene's 300 lx.

`orbe set light 10` darkens the room and the LED comes on. `orbe set light 300` brings the room back and the LED goes out.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
