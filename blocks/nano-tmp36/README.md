# TMP36 thermometer on a Nano

An Analog Devices TMP36 analog temperature sensor (`tmp36`, TO-92) is powered from the Nano's 5 V rail and feeds `A0` directly. Its output sits at 750 mV at 25 °C and rises by 10 mV/°C (`VOUT = 0.5 + 0.01 * T_C`) across -40 °C to 125 °C.

The sketch reads `A0` every 20 ms, converts the 10-bit ADC reading to volts (against the Nano's ~4.76 V USB AVCC) and degrees Celsius, and prints `voltage=<V> V, temp=<degC> C` over Serial at 115200 baud whenever the ADC reading changes.

`orbe set temperature 40` warms the room to 40 °C (`VOUT` rises from 750 mV to 900 mV and the sketch prints the new voltage and temperature).

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
