# Temperature and pressure on a DevKitC-1

The sketch is Adafruit's BMP280 example. It calls `begin(0x76)` because this module pulls SDO to ground, then `readTemperature` and `readPressure`, and prints both on Serial.

The module has no regulator and runs from 1.71 V to 3.6 V. On the ESP32-S3-DevKitC-1, VCC goes straight to 3V3, GND to GND, and SDA and SCL to Wire's default pins on this board, GPIO8 and GPIO9.

`orbe set temperature` and `orbe set pressure` change what it prints. Pressure is in Pa, and 101325 is sea level.

Rebuild `firmware/bmp280/bmp280.ino` for `esp32:esp32:esp32s3` with Adafruit BMP280 3.0.0, Adafruit BusIO 1.17.4 and Adafruit Unified Sensor 1.1.15, the libraries the image was built with. `firmware/NOTICE` names those archives.

SPI, 3-wire SPI and the NVM copy time are not modelled. Pulling CSB low warns `spi`.

The stock sketch warns `config_in_normal` because Adafruit_BMP280 3.0.0 `setSampling()` writes `config` (0xF5) before `ctrl_meas` (0xF4) after `begin()` leaves the chip in normal mode (§4.3.5). With that library, call `setSampling()` first with `MODE_SLEEP`, wait until `(bmp.getStatus() & 0x08) == 0` for the in-progress conversion to finish, and then call `setSampling()` with the wanted mode; the second call's `config` write lands in sleep mode, and expect the one `config_in_normal` warning from the first call.

Check it with `orbe check scene.jsonc` and run its tests with `orbe test`.
