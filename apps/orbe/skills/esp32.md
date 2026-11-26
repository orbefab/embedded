# orbe: esp32

orbe never compiles. Build an `esp32s3-devkitc-1` sketch with arduino-cli and name its app and its partition table in the scene.

## Install

`arduino-cli core list` should list `esp32:esp32`. If not:

```sh
arduino-cli config add board_manager.additional_urls https://espressif.github.io/arduino-esp32/package_esp32_index.json
arduino-cli core update-index
arduino-cli core install esp32:esp32
```

## The sketch

This blinks GPIO4 every 500 ms and prints `ready`.

```cpp blink/blink.ino
const int ledPin = 4;

void setup() {
  Serial.begin(115200);
  pinMode(ledPin, OUTPUT);
  Serial.println("ready");
}

void loop() {
  digitalWrite(ledPin, HIGH);
  delay(500);
  digitalWrite(ledPin, LOW);
  delay(500);
}
```

```sh
arduino-cli compile --fqbn esp32:esp32:esp32s3 --output-dir build blink
```

`build/` holds `blink.ino.bin`, the app, and `blink.ino.partitions.bin`, the partition table. The chip starts where the bootloader leaves the app, so it needs neither `blink.ino.bootloader.bin` nor `blink.ino.merged.bin`.

## Which files

```jsonc
"board": {
  "type": "esp32s3-devkitc-1",
  "app": "build/blink.ino.bin",
  "partitions": "build/blink.ino.partitions.bin",
},
```

Wire an LED through 220 ohm from `board.GPIO4` to `board.GND`. Paths start at the scene's folder. Compile and `orbe open` again after each sketch change.

```text tests/blink.test
# The LED is lit for the first 500 ms of loop, then off.
open
step 300ms
expect @led1 lit
expect serial contains "ready"
step 500ms
expect @led1 off
```

## The board

Load `orbe skills get boards` for `esp32s3-devkitc-1` pinouts, pin functions, supply limits, and unsimulated pins (`RST` and `GPIO35` to `GPIO37`). On this v1.1 board, `@board.led` is on `GPIO38`, not `LED_BUILTIN` (`48`): call `rgbLedWrite(38, 0, 40, 0)` (its 0-bit high is 400 ns, the SK68xx sheet's maximum, and the chain takes it).

## The chip

- `setup()` starts about 9 ms after t = 0. `millis()`, `delay()` and the timers use simulated time.
- `Serial` reaches `serial`, on UART0 by default or over USB Serial/JTAG with `CDCOnBoot=cdc` in the FQBN (arduino-esp32 3.3.x and 4.0.x). USB Serial/JTAG claims GPIO19 and GPIO20 and pulls GPIO20 up through its D+ pull-up until `pinMode` or `gpio_config` clears `USB_PAD_ENABLE`, which also detaches the USB console.
- The model runs the pins, the one-shot ADC, the internal temperature sensor, PCNT, LEDC, RMT channel 0, SPI2 and I2C0 as masters, UART0 to UART2, the timers and both cores.
- `driver/pulse_cnt.h` and `ESP32Encoder` run on all four PCNT units (two channels each) from real pin edges through the GPIO matrix, including quadrature counting, the APB glitch filter, limits, thresholds and interrupts.
- `temperatureRead()` reads the scene's ambient `world.temperature` (25 degC by default; chip self-heating is not simulated) within the configured TRM range (-40 to 125 degC).
- Hardware SHA (SHA-1/224/256/384/512 and SHA-512/t), AES-128/256 (ECB, CBC, OFB, CTR, CFB and mbedTLS AES-GCM) and RSA big-number math (up to 4096-bit) run through mbedTLS and GDMA.
- `ledcAttach`, `ledcDetach`, `ledcWrite`, `ledcFade`, `ledcFadeWithInterrupt`, `analogWrite`, `tone` and `noTone` drive the pin's PWM edges. `analogWrite` is 1 kHz at 8 bits, and `ledcAttach(pin, 50, 14)` is a 50 Hz servo signal. `step --until @board.GPIO18 high` lands on an edge. Each edge costs a circuit solve, so a kHz PWM runs several times slower than real time.
- `SPI` runs on SPI2. `SPI.transfer` returns what an SPI part answers, and `0xFF` when none does.
- `Adafruit_NeoPixel` and `rmtWrite` send each frame over RMT, and every pixel of a chained stick or strip takes its colour. Wire DIN straight to the pin: the frame does not cross a series resistor.
- A stock sketch's boot clears the radio's force-up bits and turns on the APP CPU's debug trace, and `gpio_config` on an RTC pad reads the pad word and leaves it on digital GPIO. It also resets the system timer, the timer groups and the RTC watchdog, and the model runs those. Those accesses are quiet, so `expect no diagnostics` passes. Another register the model does not keep, in a peripheral it runs, is `warning[chip.unmodeled_register]`.
- `orbe add devkitc-button-led` is the DevKitC-1. A button on GPIO4 cycles `@board.led`, and each colour is printed on `serial`.
- `orbe add devkitc-gy-bmp280` reads a `gy-bmp280` over I2C at 3.3 V on Wire's default pins (GPIO8 SDA, GPIO9 SCL) and prints temperature and pressure on `serial`.
- Keep PSRAM off, the FQBN's default. With `PSRAM=opi` the run stops at boot with `error[chip.flash_command]`.
- A peripheral the model does not run stops the run at its first access with `error[chip.unsupported_setting]` that names it, as in `used MCPWM0, the motor-control PWM that ESP32Servo drives servos with`: Wi-Fi and Bluetooth, MCPWM (ESP32Servo; drive a servo with `ledcAttach` instead), I2S, SPI3, `Wire1`, TWAI and USB OTG. `touchRead` returns before it reaches the touch sensor, so nothing flags it.
- Flash writes work on the 16 MB SPI flash with typical program and erase times, so `Preferences`, `EEPROM`, `nvs_flash_init` and `LittleFS` save and read back data. Status register writes still stop the run with `error[chip.flash_command]`.
- An unmodelled setting stops the run with `error[chip.unsupported_setting]` and names the peripheral and the setting.
