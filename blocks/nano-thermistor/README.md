# Thermistor thermometer on a Nano

A 10 kΩ NTC thermistor (`thermistor-ntc-10k`, Vishay NTCLE100E3103JB0, B25/85 = 3977 K) sits above a 10 kΩ axial resistor from 5 V to ground, feeding `A0` on an Arduino Nano. As temperature rises, the thermistor's resistance falls and `A0` rises.

The sketch converts the ratiometric ADC reading to °C with the B equation, prints `temp=<degC> C` over Serial at 115200 baud whenever the rounded reading changes, and lights the red LED on `D9` at or above 30 °C.

`orbe set temperature 40` warms the room to 40 °C (the thermistor drops to about 5.28 kΩ and the LED lights). `orbe set temperature 10` cools it to 10 °C (about 20.23 kΩ and the LED turns off).

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
