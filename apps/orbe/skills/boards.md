# orbe: boards

Exact pins, on-board wiring, simulated functions, supplies and unsimulated pins for shipped boards. `orbe part <id>` prints a full card.

### `nano-board`

- Nano (CH340 clone), ATmega328P-AU (`mcu`), 16 MHz. Two 1x15 rows straddle 15 breadboard columns.
- Pins `j1` (1-15): D1, D0, RESET, GND, D2-D12. `j2` (1-15): D13, 3V3, AREF, A0-A7, 5V, RESET, GND, VIN. These pins work: all 30. Seat `on` by a single-leg pin like `D13` (RESET, GND have 2 pins each).
- On-board: D13 (PB5) drives yellow `led_l` (`@nano.led_l`) via `r_l` (1 kohm); red `led_pwr` on 5V via 1 kohm; RESET (PC6) has `sw_reset` to GND, 10 kohm `r_reset` to 5V; D0 joins CH340G TXD (idles 5 V) via 1 kohm `r_txd`; D1 UART0 TX (`serial`).
- Functions: digital I/O D0-D13, A0-A5 (`INPUT`, `INPUT_PULLUP`, `OUTPUT`); 10-bit ADC A0-A7 (ch 8 temp); PWM D3, D5, D6, D9, D10, D11; I2C master A4 (SDA), A5 (SCL); SPI D10 (SS), D11 (MOSI), D12 (MISO), D13 (SCK); UART0 D0 (RX), D1 (TX); interrupts INT0 (D2), INT1 (D3), PCINT D0-D13, A0-A5.
- Roles: D0/D1 UART0; D13 pulls `INPUT_PULLUP` to ~1.45 V via `led_l` (reads low); A6/A7 analog-only; AREF open; RESET active-low.
- Supplies: USB 5 V (500 mA peak) feeds 5V via MBR0520 Schottky (0.5 A avg, 5.5 A peak; 5V ~4.76 V idle). VIN feeds AMS1117-5.0 (15 V max, >= 6.1 V holds 5V at 5.0 V, 1 A / 1.1 W, limit 1.1 A; 0 V USB-only). 3V3 is CH340G V3 (3.3 V with 5 V up; warns `regulator.current_limit` > 30 mA). MCU (10.2 ms RMS): 40 mA/pin (<= 20 mA for clean levels), 200 mA VCC/GND, -0.5 V to VCC + 0.5 V (RESET 13 V), VCC 6 V.
- Not simulated: USB data, `Serial.read()`, DTR auto-reset, `led_tx`/`led_rx`, rail caps, AMS1117 reverse diode (real board ~4 V on VIN from USB), clamp diodes, output saturation, analog comparator, I2C slave, Timer2 async, `SPM`.

### `uno-board`

- Uno R3 compatible (CH340G clone), ATmega328P-PU (`mcu`, DIP-28), 16 MHz. Female headers sit beside a breadboard with `at`.
- Pins: D0-D13, GND (x3), AREF, SDA (= A4), SCL (= A5), IOREF (= 5V), RESET, 3V3, 5V, VIN, A0-A5. These pins work: D0-D13, GND, AREF, SDA, SCL, IOREF, RESET, 3V3, 5V, A0-A5.
- On-board: D13 (PB5) drives yellow `led_l` (`@uno.led_l`) directly via `r_l` (1 kohm, unbuffered, unlike official Uno R3); green `led_on` on 5V via 1 kohm; RESET has `sw_reset`, 10 kohm `r_reset`, `d_reset`, 100 nF `c_reset`; D0 joins CH340G TXD via 1 kohm `r_txd`; D1 UART0 TX; 100 nF `c_aref` on AREF.
- Functions and roles: same ATmega328P functions/roles as `nano-board`, with 6 ADC pins (A0-A5; no A6/A7). `INPUT_PULLUP` on D13 reads low (~1.45 V, unbuffered `led_l`).
- Supplies: 5V/IOREF from USB 5 V (500 mA peak) via 0.15 ohm PTC, FDN340P PMOS (2 A / 0.5 W). 3V3 from `reg_3v3` (LP2985-33, 150 mA / 0.49 W, limit 350 mA). MCU ratings match Nano.
- Not simulated: `VIN` (The barrel jack feeds it. The board runs from USB. Wire to it is `not_simulated`), barrel jack, 5 V regulator, USB/VIN switchover, PTC trip, USB data, `Serial.read()`, `led_tx`/`led_rx`, rail caps, Nano's ATmega328P gaps.

### `esp32s3-devkitc-1`

- ESP32-S3-DevKitC-1 v1.1, ESP32-S3-WROOM-1-N16R8 (`mod`, 16 MB quad flash, 8 MB octal PSRAM). Two 1x22 headers (`j1`, `j3`).
- Pins `j1` (1-22): 3V3 (x2), RST, GPIO4-7, 15-18, 8, 3, 46, 9-14, 5V, GND. `j3` (1-22): GND, TX/GPIO43, RX/GPIO44, GPIO1, 2, 42-35, 0, 45, 48, 47, 21-19, GND (x2). Name GPIOs `GPIO<n>`. These pins work: 3V3, 5V, GND, TX/GPIO43, RX/GPIO44, GPIO0-21, GPIO38-48. Seat `on` by one pin like `GPIO8`.
- RGB LED: `GPIO38` drives `@board.led` (`sk68xxmini-hs` on 3V3). Arduino-ESP32 (`esp32s3`) sets `LED_BUILTIN`/`RGB_BUILTIN` to `SOC_GPIO_PIN_COUNT + 48` (GPIO48, v1.0), so stock Blink (`digitalWrite(LED_BUILTIN, HIGH)` or `rgbLedWrite(RGB_BUILTIN, ...)`) lights nothing. Use `rgbLedWrite(38, r, g, b)` (e.g. `rgbLedWrite(38, 0, 40, 0)`) or `Adafruit_NeoPixel` on 38.
- On-board: GPIO0 joins `sw_boot` (BOOT to GND), `q2`; TX/GPIO43, RX/GPIO44 join CP2102N (`j2`; TXD idles 3.3 V on GPIO44); GPIO19 (D-), GPIO20 (D+) join USB `j4` and `USB_SERIAL_JTAG`.
- Functions: digital I/O and interrupts on every simulated GPIO, ADC1 on GPIO1-10 (A0-A9) and ADC2 on GPIO11-20 (A10-A19), and peripherals routed to any simulated GPIO. Which peripherals run is in `orbe skills get esp32`. Arduino defaults: Wire GPIO8 (SDA), GPIO9 (SCL); SPI GPIO10 (SS), GPIO11 (MOSI), GPIO12 (SCK), GPIO13 (MISO); `Serial` GPIO43/44 (or USB with `CDCOnBoot=cdc`); `Serial1` GPIO16 (TX), GPIO15 (RX); `Serial2` GPIO20 (TX), GPIO19 (RX).
- Roles: strapping GPIO0, 3, 45, 46 act as normal GPIOs (orbe skips boot ROM; `sw_boot` on GPIO0). At reset `USB_SERIAL_JTAG` claims GPIO19/20 with 2.4 kohm D+ pull-up on GPIO20 until `pinMode`/`gpio_config` clears `USB_PAD_ENABLE` (detaching USB console). GPIO39-42 default to JTAG on hardware, normal GPIOs once configured. GPIO26-32 (not on headers) are flash (`chip.flash_command`).
- Supplies: 5V is `j2.VBUS` (5.0 V, 500 mA peak USB, no diode drop). 3V3 is SGM2212-3.3 LDO (800 mA / 0.85 W, limit 1.1 A; fed by 500 mA `j2`). Chip: VCC max 3.6 V, GPIO -0.3 V to VCC + 0.3 V (3.3 V logic; `bss138-level-shifter` for 5 V); pin/supply currents unrated.
- Not simulated on the board: `RST` (no reset pin), `GPIO35`, `GPIO36`, `GPIO37` (octal PSRAM; wiring these is `not_simulated`), `j4` 5 V, power LED D5, clamp diodes. Chip features that do not run yet are listed in `orbe skills get esp32`.

### `breadboard-400` and `breadboard-830`

- Both `joins_only` (ideal nets, no resistance/ratings; 8 mm hole depth; dovetails/adhesive not simulated).
- Strips: 5-hole cols across channel, `a`-`e` (`-Y`), `f`-`j` (`+Y`). Rails: `W` (+), `X` (-) beside `a`; `Y` (+), `Z` (-) beside `j`. Continuous end to end (no split).
- `breadboard-400`: 30 cols (`a1`-`j30`, 82.6 x 54.6 mm). 25 holes/rail: 1-5, 7-11, 13-17, 19-23, 25-29 (no 6, 12, 18, 24, 30).
- `breadboard-830`: 63 cols (`a1`-`j63`, 165.1 x 54.6 mm). 50 holes/rail: 3-7, 9-13, 15-19, 21-25, 27-31, 33-37, 39-43, 45-49, 51-55, 57-61 (gaps 8, 14..56; no 1, 2, 62, 63).
