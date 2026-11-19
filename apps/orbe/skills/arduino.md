# orbe: arduino

orbe runs the firmware you build. It never compiles anything. For the `nano-board` and the `uno-board`, build with arduino-cli, then name the `.hex` it writes in the scene. `orbe skills get core` covers everything else.

## Install arduino-cli

First check that `arduino-cli version` runs and that `arduino-cli core list` lists `arduino:avr`.

If it is missing, install arduino-cli as https://arduino.github.io/arduino-cli/latest/installation/ says, then the AVR core:

```sh
arduino-cli core update-index
arduino-cli core install arduino:avr
```

## Write and build the sketch

arduino-cli wants the sketch in a folder of the same name, as `button/button.ino`. This sketch lights D13 while D2 is high, for the scene in `orbe skills get core`.

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

Build it from the project folder. The Nano's FQBN is `arduino:avr:nano`. For the Uno, use `arduino:avr:uno` instead.

```sh
arduino-cli compile --fqbn arduino:avr:nano --output-dir build button
```

`build/` then holds `button.ino.hex`, `button.ino.with_bootloader.hex`, `button.ino.with_bootloader.bin`, `button.ino.elf` and `button.ino.eep`.

The names follow the sketch folder, so the firmware is `build/<sketch>.ino.hex`.

## Name the right file

The scene names `build/button.ino.hex`, the program alone:

```jsonc
"nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
```

- Not `button.ino.eep`. It is EEPROM data, and for a sketch with no EEPROM data it holds no data record. `orbe check` refuses it with `error[scene.param]`, which says the firmware `has no data record, so the flash would be erased`.
- Not `.elf` or `.bin`. `orbe check` refuses them as "not text".
- Not `button.ino.with_bootloader.hex`. It runs, but it carries the bootloader too.

The path starts from the scene's folder. After each change to the sketch, compile again and run `orbe open` again, which reads the new file. A session keeps the firmware it opened with.

`cpu=atmega328old` in the FQBN changes only the bootloader used for upload. It builds the same `.hex`, so the simulation does not need it.

## The simulated Nano and Uno

<!-- boards nano-board uno-board -->
- `millis()`, `delay()` and the timers run on simulated time, so `delay(1000)` takes `step 1s`.
- D0 to D13 and A0 to A5 take `INPUT`, `INPUT_PULLUP` and `OUTPUT`.
- An input that nothing drives keeps its last level here. On a real board it floats. Once the sketch reads it while it floats, `step` warns `chip.floating_input`, naming the pin, as in `#nano__mcu.pin.PD2 (nano.D2) floats on #net.nano__D2 at ... ps, and the firmware has taken in its level`. Give each button a pull-down resistor, as the core example does, or use `INPUT_PULLUP` and wire the button to GND. A `digitalRead` under `INPUT_PULLUP` reads the pull-up, so it does not count once `pinMode(INPUT)` turns the pull-up off. `digitalRead` loads its pin's whole port, so another `INPUT` pin of that port warns too when a part, even a capacitor, is wired to it and nothing holds it.
- `get` and `expect` read the L LED named above. With the part named `nano` or `uno`, that is `@nano.led_l` or `@uno.led_l`.
- With `INPUT_PULLUP` on D13 the LED pulls the pin to about 1.45 V, so it reads low, where an Uno that buffers D13 through an op-amp reads high. Use another pin for an input with its pull-up.
- `Serial` output reaches orbe as text. After `Serial.begin(115200)` and `Serial.println("ready")`, `expect serial contains "ready"` passes. `println` ends a line with `\r\n`, and `step` lists the D1 edges it makes. Serial can reach orbe up to 10 ms after the chip sends it. A `println` in `loop` prints on every pass, so its text repeats.
- Nothing sends serial into the chip. `Serial.read()` never gets a byte.
- The program starts at t = 0 with no bootloader delay.
- `SPI.transfer` returns what MISO (D12) reads, so wire D12 to the part that answers. With D12 tied to GND it returns 0, tied to 5V it returns 255, and wired to MOSI (D11) it returns the byte it sent.
- The chip's temperature sensor reads the scene's `world` temperature, and 25 °C when the scene sets none. `analogRead` can't reach it, because the core keeps only channels 0 to 7, so a sketch sets `ADMUX = 0xC8` (the 1.1 V reference and channel 8), starts a conversion and reads `ADC`.
- A sketch that reaches a part of the chip the simulation does not run stops the run with an error that names the peripheral and the setting, such as `error[chip.unsupported_setting]` with `#nano__mcu read the AC output while the comparator is on`. The rows before it stand. The analog comparator (`ACSR` read, or its interrupt, capture or bandgap set), Timer2 on a 32 kHz crystal (`ASSR`), self-programming (`SPMCSR`), moving the vectors (`IVSEL`), synchronous or reserved USART modes, reserved ADC references or channels, a reserved sleep mode, and a flash word that is no instruction each stop it. `ACSR |= _BV(ACD)`, which turns the comparator off, runs. Any other read of `ACSR` while the comparator is on stops the run, such as `ACSR &= ~_BV(ACD)` or `ACSR |= _BV(ACI)`.
- `Wire.begin(address)` makes the chip an I2C slave, which the simulation does not run. `step` warns `chip.unmodeled_register` on `TWAR` once, and nothing answers that address. `Wire.begin()` with no address is a master and runs. A write to `OSCCAL`, `analogRead(A6)` or `analogRead(A7)`, which read 0, and an I2C clock below about 18 kHz each warn the same way once, on `OSCCAL`, `ADMUX` or `TWBR`, and the run goes on.
