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

- `esp32s3-devkitc-1`, the DevKitC-1 v1.1 with an N16R8 module. `orbe part esp32s3-devkitc-1` lists its pins.
- `TX` and `RX` are GPIO43 and GPIO44, wired to the USB-UART bridge.
- GPIO38 drives the board's RGB LED, `@board.led`. `rgbLedWrite(38, 0, 40, 0)` latches `rgb(0,40,0)`. Its 0-bit high is 400 ns, the SK68xx sheet's maximum, and the chain takes it.
- `RST` and GPIO35 to GPIO37 are not simulated. A wire to them is `not_simulated`.

## The chip

- `setup()` starts about 9 ms after t = 0. `millis()`, `delay()` and the timers use simulated time.
- `Serial` reaches `serial`, on UART0 by default or over USB with `CDCOnBoot=cdc` in the FQBN.
- The model runs the pins, the one-shot ADC, LEDC, RMT channel 0, SPI2 and I2C0 as masters, UART0 to UART2, the timers and both cores.
- `ledcAttach`, `ledcDetach`, `ledcWrite`, `ledcFade`, `ledcFadeWithInterrupt`, `analogWrite`, `tone` and `noTone` drive the pin's PWM edges. `analogWrite` is 1 kHz at 8 bits, and `ledcAttach(pin, 50, 14)` is a 50 Hz servo signal. `step --until @board.GPIO18 high` lands on an edge. Each edge costs a circuit solve, so a kHz PWM runs several times slower than real time.
- `SPI` runs on SPI2. `SPI.transfer` returns what an SPI part answers, and `0xFF` when none does.
- `Adafruit_NeoPixel` and `rmtWrite` send each frame over RMT, and every pixel of a chained stick or strip takes its colour. Wire DIN straight to the pin: the frame does not cross a series resistor.
- A stock sketch's boot clears the radio's force-up bits and turns on the APP CPU's debug trace, and `gpio_config` on an RTC pad reads the pad word and leaves it on digital GPIO. It also resets the system timer, the timer groups and the RTC watchdog, and the model runs those. Those accesses are quiet, so `expect no diagnostics` passes. An access that sketch does not make is still `warning[chip.unmodeled_register]`.
- `orbe add devkitc-button-led` is the DevKitC-1. A button on GPIO4 cycles `@board.led`, and each colour is printed on `serial`.
- Keep PSRAM off, the FQBN's default. With `PSRAM=opi` the run stops at boot with `error[chip.flash_command]`.
- Wi-Fi and Bluetooth are not simulated.
- An unmodelled setting stops the run with `error[chip.unsupported_setting]` and names the peripheral and the setting.
