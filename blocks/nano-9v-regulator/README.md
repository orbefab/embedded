# 9 V battery and LM7805 regulator on a Nano

A 9 V PP3 alkaline battery (`battery-9v`, 9.0 V EMF, 1.7 Ω Duracell MN1604 1 kHz impedance, 50 mA characterized continuous load on Energizer 522) powers an `lm7805-to220` 5 V regulator (lighting a green LED through 470 Ω) and the Nano's `VIN` pin. A 10 kΩ / 10 kΩ divider brings half of the battery voltage into `A0`, and the Nano prints `bat=<V>V` over Serial.

Idle (about 43 mA total draw, within Energizer 522's 50 mA continuous bar), the battery sits near 8.93 V with no warnings. Holding `btn1` switches a 25 Ω `resistor-sqp15` load across the battery (~376 mA total), sagging the 9 V rail to about 8.36 V (the ~0.57 V drop predicted by 1.7 Ω internal resistance) while both the LM7805 and Nano 5 V rails hold at 5.00 V and `battery-9v` warns `rating.current` that the load is beyond the 50 mA load its sheet characterizes.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
