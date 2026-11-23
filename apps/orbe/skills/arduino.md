# orbe: arduino

orbe never compiles. Build a `nano-board` or `uno-board` with arduino-cli and name the `.hex` in the scene.

## Install

`arduino-cli version` should run, and `arduino-cli core list` should list `arduino:avr`. If not, install it as https://arduino.github.io/arduino-cli/latest/installation/ says, then:

```sh
arduino-cli core update-index
arduino-cli core install arduino:avr
```

## The sketch

The sketch folder has the same name. This lights D13 while D2 is high, for the scene in core.

```cpp button/button.ino
const int buttonPin = 2;
const int ledPin = 13;

int buttonState = 0;

void setup() {
  pinMode(ledPin, OUTPUT);
  pinMode(buttonPin, INPUT);
}

void loop() {
  buttonState = digitalRead(buttonPin);
  if (buttonState == HIGH) {
    digitalWrite(ledPin, HIGH);
  } else {
    digitalWrite(ledPin, LOW);
  }
}
```

The Nano's FQBN is `arduino:avr:nano`. For the Uno, use `arduino:avr:uno`.

```sh
arduino-cli compile --fqbn arduino:avr:nano --output-dir build button
```

`build/` holds `button.ino.hex`, `button.ino.with_bootloader.hex`, `button.ino.with_bootloader.bin`, `button.ino.elf` and `button.ino.eep`. The firmware is `build/<sketch>.ino.hex`.

orbe runs a stock library, such as `arduino-cli lib install "DHT sensor library"` or `"Adafruit NeoPixel"`.

## Which file

```jsonc
"nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
```

- Not `button.ino.eep`. With no EEPROM data it has no data record. `orbe check` reports `error[scene.param]`: the firmware `has no data record, so the flash would be erased`.
- Not `.elf` or `.bin`. `orbe check` refuses them as "not text".
- Not `button.ino.with_bootloader.hex`. It runs, but it carries the bootloader too.

Paths start at the scene's folder. Compile and `orbe open` again after each sketch change. A session keeps that firmware. `cpu=atmega328old` changes only the upload bootloader. The `.hex` stays the same.

## A Block

A Block's sketch is `firmware/<name>/<name>.ino` and the scene names `build/<name>.ino.hex`. `<name>` is the folder, `fade` for `nano-led-fade` and `button` for `nano-button-led`. A Uno Block uses FQBN `arduino:avr:uno`.

```sh
arduino-cli compile --fqbn arduino:avr:nano --output-dir build firmware/fade
```

A Block's tests check what it shipped. Update them when the timing or the pins change.

### `nano-board`

- `nano-board`, Nano (CH340 clone), runs the ATmega328P-AU at 16 MHz.
- These pins work: `D0` to `D13`, `RESET`, `GND`, `AREF`, `A0` to `A7`, `5V`, `3V3`, and `VIN`.
- `3V3` is the CH340G's V3 pin. With 5 V it holds about 3.3 V. Past 30 mA it warns `rating.current`. With no 5 V it is off.
- `VIN` feeds `regulator`, an AMS1117-5.0. About 6.1 V or more on VIN holds 5V at 5.0 V. With VIN open the board runs from USB and 5V stays that rail. On USB only, VIN reads 0 V and supplies nothing. A real board shows about 4 V there, through the regulator's internal diode.
- `D13` drives `led_l` through `r_l`, 1 kohm.

### `uno-board`

- `uno-board`, Uno R3 compatible board, runs the ATmega328P-PU at 16 MHz.
- These pins work: `D0` to `D13`, `GND`, `AREF`, `SDA`, `SCL`, `IOREF`, `RESET`, `3V3`, `5V`, and `A0` to `A5`.
- `SDA` and `A4` are one net.
- `SCL` and `A5` are one net.
- `IOREF` and `5V` are one net.
- `3V3` comes from `reg_3v3`, a 3.3 V regulator.
- `VIN` is not simulated. The barrel jack feeds it. The board runs from USB. A wire to it is a `not_simulated` error.
- `D13` drives `led_l` through `r_l`, 1 kohm.

## The chip

- `millis()`, `delay()` and the timers use simulated time. `delay(1000)` takes `step 1s`. The program starts at t = 0, with no bootloader delay.
- D0 to D13 and A0 to A5 take `INPUT`, `INPUT_PULLUP` and `OUTPUT`. A Nano's A6 and A7 are analog only. `analogRead` reads them.
- An undriven input keeps its last level. A real pin floats. Once the sketch reads it floating, `step` warns `chip.floating_input`, as in `#nano__mcu.pin.PD2 (nano.D2) floats on #net.nano__D2 at ... ps, and the firmware has taken in its level`. Use a pull-down, as in core, or `INPUT_PULLUP` to GND. Under `INPUT_PULLUP`, `digitalRead` reads the pull-up, so it stops once `pinMode(INPUT)` turns the pull-up off. `digitalRead` loads the whole port, so another `INPUT` pin warns too when a part, even a capacitor, is on it and nothing holds it.
- `@nano.led_l` and `@uno.led_l` are the L LED. `INPUT_PULLUP` on D13 pulls the pin to about 1.45 V through that LED, so it reads low. An Uno that buffers D13 through an op-amp reads high. Use another pin for a pull-up input.
- After `Serial.begin(115200)` and `Serial.println("ready")`, `expect serial contains "ready"` passes. `println` ends with `\r\n`, and `step` lists the D1 edges. Text can arrive one engine step late (`sim.max_step`, 10 ms by default). A `println` in `loop` repeats. `Serial.read()` never gets a byte.
- `SPI.transfer` returns MISO (D12). Wire D12 to the part that answers. GND returns 0, 5V returns 255, and MOSI (D11) returns the byte sent.
- The temperature sensor reads `world` temperature, or 25 °C when unset. `analogRead` has channels 0 to 7 only, so set `ADMUX = 0xC8` (1.1 V reference, channel 8), start a conversion and read `ADC`.
- An unmodelled setting stops the run with `error[chip.unsupported_setting]` and names the peripheral, as in `#nano__mcu read the AC output while the comparator is on`. Earlier rows stand. These stop it: `ACSR` while the comparator is on (a read, its interrupt, capture or bandgap); Timer2 on a 32 kHz crystal (`ASSR`); `SPMCSR`; `IVSEL`; a synchronous or reserved USART mode; a reserved ADC reference or channel; a reserved sleep mode; a flash word that is no instruction. `ACSR |= _BV(ACD)` turns the comparator off and runs. Any other `ACSR` read while it is on stops the run, such as `ACSR &= ~_BV(ACD)` or `ACSR |= _BV(ACI)`.
- `Wire.begin(address)` is an I2C slave and is not run. `step` warns `chip.unmodeled_register` on `TWAR` once, and nothing answers. `Wire.begin()` with no address is a master and runs. A write to `OSCCAL`, or an I2C clock below about 18 kHz, warns once (`OSCCAL` or `TWBR`) and the run goes on.
