# Buzzer on a Nano

D8 plays 440 Hz for 200 ms, rests, plays 880 Hz for 200 ms and rests. A 220 ohm resistor is in series with the buzzer, because the coil is 42 ohm and a pin wired straight across it exceeds the pin's 40 mA rating.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
