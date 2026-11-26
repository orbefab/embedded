# Thermistor thermometer on a Nano

A 10 kΩ NTC thermistor (`thermistor-ntc-10k`, Vishay NTCLE100E3103JB0, following the datasheet's R/T curve from -40 to 125 °C) sits above a 10 kΩ axial resistor from 5 V to ground, feeding `A0` on an Arduino Nano. As temperature rises, the thermistor's resistance falls and `A0` rises.

The sketch converts the ratiometric ADC reading to °C with the B equation (B25/85 = 3977 K, which tracks the full R/T curve within about 0.4 °C between 10 and 40 °C), prints `temp=<degC> C` over Serial at 115200 baud whenever the rounded reading changes, and lights the red LED on `D9` at or above 30 °C.

`orbe set temperature 40` warms the room to 40 °C (the thermistor drops to 5.33 kΩ, the sketch prints `temp=39.7 C`, and the LED lights). `orbe set temperature 10` cools it to 10 °C (19.87 kΩ, `temp=10.4 C`, and the LED turns off).

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
