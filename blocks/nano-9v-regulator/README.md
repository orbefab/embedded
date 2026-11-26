# 9 V battery and LM7805 regulator on a Nano

A 9 V PP3 alkaline battery (`battery-9v`, 9.0 V EMF, 1.7 Ω internal resistance, 500 mA rating) powers an `lm7805-to220` 5 V regulator (lighting a green LED through 220 Ω) and the Nano's `VIN` pin. A 10 kΩ / 10 kΩ divider brings half of the battery voltage into `A0`, and the Nano prints `bat=<V>V` over Serial.

Idle (about 45 mA total draw), the battery sits near 8.92 V. Holding `btn1` switches a 25 Ω `resistor-sqp15` load across the battery (~375 mA total), sagging the 9 V rail to about 8.36 V (the ~0.56 V drop predicted by 1.7 Ω internal resistance) while both the LM7805 and Nano 5 V rails hold at 5.00 V.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
