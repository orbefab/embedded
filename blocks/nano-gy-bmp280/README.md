# Temperature and pressure on a Nano

The sketch is Adafruit's BMP280 example. It calls `begin(0x76)` because this module pulls SDO to ground, then `readTemperature` and `readPressure`, and prints both on Serial.

The module has no regulator, so its VCC is on the Nano's 3V3 pin. Its SDA and SCL are 3.3 V lines and the Nano's are 5 V, so a BSS138 level shifter joins them: HV to 5V, LV to 3V3, A4 to HV1 and A5 to HV2. Wired straight to A4 and A5, the Nano's pull-ups lift SDA and SCL to about 3.6 V. The sensor still reads, and orbe warns `rating.voltage` on both pins.

`orbe set temperature` and `orbe set pressure` change what it prints. Pressure is in Pa, and 101325 is sea level.

Rebuild `firmware/bmp280/bmp280.ino` with Adafruit BMP280 3.0.0, Adafruit BusIO 1.17.4 and Adafruit Unified Sensor 1.1.15, the libraries the image was built with. `firmware/NOTICE` names those archives.

SPI, 3-wire SPI and the NVM copy time are not modelled. Pulling CSB low warns `spi`.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
