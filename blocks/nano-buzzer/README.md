# Buzzer on a Nano

D8 plays 440 Hz for 200 ms, rests, plays 880 Hz for 200 ms and rests. A 2N2222 switches the buzzer from the Nano's 5 V pin through 10 ohm, so the coil sees about 3.5 V, inside its 3 V to 5 V operating window. D8 only drives the base, through 1 kohm.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
