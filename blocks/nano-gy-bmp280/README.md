# Temperature and pressure on a Nano

The sketch is Adafruit's BMP280 example. It calls `begin(0x76)` because this module pulls SDO to ground, then `readTemperature` and `readPressure`, and prints both on Serial. An LP2985 makes 3.3 V for the module from the Nano's 5V. The module itself has no regulator. A 68 kΩ from SDA to ground and another from SCL to ground keep the Nano's 5 V I2C pull-ups from feeding that regulator.

`orbe set temperature` and `orbe set pressure` change what it prints. Pressure is in Pa, and 101325 is sea level.

Rebuild `firmware/bmp280/bmp280.ino` with Adafruit BMP280 3.0.0, Adafruit BusIO 1.17.0 and Adafruit Unified Sensor 1.1.15, the libraries the image was built with. `firmware/NOTICE` names those archives.

SPI, 3-wire SPI and the NVM copy time are not modelled. Pulling CSB low warns `spi`.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
