# SG90 servo sweep on a Nano

D9 drives an SG90. The sketch writes 0° to 180° and back, one degree every 15 ms, and starts over, so the horn keeps sweeping. The servo stands beside the breadboard, and three jumpers run from its connector: SIGNAL to D9, VCC to the 5 V rail and GND to the ground rail. The servo's current is on the Nano's 5 V pin.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
