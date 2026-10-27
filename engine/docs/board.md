# Development board

`examples/atmega-board-blink` is a common ATmega328P development board, revision 3, cut down to parts the engine already runs. Copy its `netlist.json`, replace `mcu.firmware` with the Intel HEX text of a sketch, and append the parts and nets that sketch needs. Header nets use the labels on the header. They are `d0` to `d13`, `a0` to `a5`, `aref`, `reset`, `v5`, `v3v3`, and `gnd`. `a4` is the SDA header pin and `a5` is the SCL header pin. Internal nets keep a schematic name where that name is legal. Those are `usbvcc`, `dtr`, `m8rxd`, `xtal1`, and `xtal2`. Nets the schematic leaves unnamed take a role name. Those are `vbus`, `dtr_pin`, `l13`, and `on_led`. Parts take role names such as `mcu` and `r_l`, so a later `r1` or `led1` does not collide with them. Each value's `at` names the schematic designator. No value has source `user`.

The schematic is [https://www.arduino.cc/en/uploads/Main/Arduino_Uno_Rev3-schematic.pdf](https://www.arduino.cc/en/uploads/Main/Arduino_Uno_Rev3-schematic.pdf). Its SHA-256 is `665fe3c80f913f0e4a39752bd03a5ec3c4c7e50bf122be781b2571e8ef4d0e2d`. Values that come off that sheet cite `development board schematic, revision 3`.

The example runs `firmware/sketch/blink.hex` for 4 s on a 10 ms grid and writes 401 rows. `mcu` uses the viewer id `dev-board-328p`. Every other part uses `on-board`. A session drives the reset button with `applyContact` on `sw_reset`, drives DTR with `applyLevel` on `usb_dtr`, and sends host bytes with `applySerial` on `usb_tx`. `crates/cli/tests/board.rs` opens this example, advances to the input, sets `usb_dtr` to 0 V through that level input, and checks that the sketch restarts.

## Parts

The fragment is on USB power. `v5` is the 5 V header pin, and it is also the IOREF pin, which the sheet ties to +5V. `v3v3` is an ideal 3.3 V source. The LP2985-33 is specified for 150 mA. Its peak output current is typically 350 mA, and the minimum of that peak is 300 mA. The model has no current limit. A load on `v3v3` does not load `v5`. On the board the regulator draws that current from +5V.

| Part | Designator | What it is |
| --- | --- | --- |
| `usb_vbus` | host VBUS | 5.0 V on `vbus` |
| `usb_fuse` | F1 | 0.15 ohm from `vbus` to `usbvcc`. The sheet names MF-MSMF050-2. The `-2` suffix is tape and reel packaged per EIA-481, which is why the MF-MSMF050 row is this part. That row's minimum resistance at 23 C is 0.15 ohm. The fuse holds 0.5 A and trips at 1.0 A. This resistor never trips. |
| `usb_switch` | T1, FDN340P | 0.06 ohm from `usbvcc` to `v5`, the typical on resistance at VGS of -4.5 V. The gate drive and the comparator that produces it are left out, so the switch stays on. |
| `reg_3v3` | U2, LP2985-33 | 3.3 V on `v3v3` |
| `mcu` | ZU4, ATmega328P | The documented pin values. `clock_hz` is 16 MHz. Fuses are low `0xFF`, high `0xDF`, extended `0xFD`. |
| `res_xtal1`, `res_xtal2` | resonator | 15 pF each, held at 0 V, so `xtal1` and `xtal2` have a DC path |
| `c_aref` | C4 | 100 nF from `aref` to `gnd`, held at 0 V |
| `usb_tx` | 16U2 TX | A serial source on `m8rxd`, mark 5 V, space 0 V, no events |
| `r_rx` | RN4B | 1 kohm from `m8rxd` to `d0` |
| `usb_dtr` | 16U2 PD7 | A steps source on `dtr_pin`, held at 5 V |
| `dtr_out` | U3 PD7 | 36 ohm from `dtr_pin` to `dtr`. At 25 C and 5 V, Figure 27-11 reads about 0.68 V at a 20 mA sink, and Figure 27-13 reads about 4.20 V at a 20 mA source, a 0.80 V drop. Those are about 34 ohm and 40 ohm. One resistor is 36 ohm, inside that range. The plots end at 20 mA, and the clamp current is past that axis. |
| `r_dtr_boot` | RN2D | 1 kohm from `dtr` to `gnd`, labelled USB boot En |
| `c_reset` | C5 | 100 nF from `dtr` to `reset`, initial voltage unset. RESET-EN is closed, so the jumper is this wire. |
| `r_reset` | RN1D | 10 kohm from `reset` to `v5` |
| `d_reset` | D2, CD1206-S01575 | Diode, anode on `reset`, cathode on `v5`. `Is`, `N`, and `Rs` are the 25 C forward curve through (0.82 V, 10 mA), (0.97 V, 100 mA), and (1.07 V, 200 mA). Current is rated at the 4 A surge, and power at 400 mW. |
| `sw_reset` | RESET button | The schematic names TS42031-160R-TR-7260. No sheet for that part number was found. The model uses 0.1 ohm closed and 1e8 ohm open, from `reset` to `gnd`, open, no events. |
| `r_l` | RN2A | 1 kohm from `d13` to `l13` |
| `led_l` | L | The red WP7113ID fit. The sheet marks this LED yellow and publishes no part number. |
| `r_on_a`, `r_on_b` | RN4D, RN4C | 1 kohm each, in parallel from `v5` to `on_led` |
| `led_on` | ON | The same red LED. The sheet marks this one green and publishes no part number. |

On a release run of the blink example, `#net.v5` is 4.998007 V while D13 is high and the host supplies 9.491 mA. While D13 is low, `#net.v5` is 4.998671 V and the host supplies 6.328 mA. `#net.v3v3` is 3.3 V. The pin-13 LED draws 3.164 mA while D13 is high. D0 idles at 4.999091 V. While U3 drives high, `dtr` rests at 4.826 V. That host current leaves out the USB-serial chip, the op-amp, and the regulator. `r_dtr_boot` draws from `usb_dtr`, not from VBUS.

## What differs from the board

D13 drives `r_l` and `led_l` directly. The op-amp follower on that pin is left out. Pin 13 as an input with its pull-up on sits at 1.454 V, so it reads low and the LED is dim. On the board that pin reads high and the LED lights. An unused pin 13 sits near 0.778 V at the operating point, and the LED is dark.

There is no bootloader. The sheet's high fuse is `0xDE`, which programs BOOTRST, and the engine rejects that byte. `0xDF` is that byte with BOOTRST unprogrammed. After an external reset the sketch starts 65 ms plus 14 cycles after the release. On the board the bootloader runs first and waits about a second. Power-on start-up is already over at cycle 0, so this sketch's first PB5 rise is 233 cycles after t = 0.

The release is the first-order step of the 100 nF capacitor, RN2D, the 36 ohm pin, the 10 kohm pull-up, the 46,296 ohm reset pull-up, the pin leakage, and the clamp's reverse current. The threshold is 0.44 of the solved `v5`. A DTR fall released 64.5 ns before that form. Opening the reset button released 69.6 ns before it. The test window is 200 ns, the larger gap plus 130 ns. PB5's next rise landed 59.4 ns and 23.2 ns before 65.0154375 ms after those releases. One cycle at 16 MHz is 62.5 ns, so both are inside one cycle. RESET has no pulse-width filter, so a glitch shorter than 2.5 us still resets.

With `dtr_out` and RN2D in place, a rising DTR edge peaks RESET at 6.002 V and the diode at 110 mA, and the run reports no rating warning. Without that 36 ohm stage, the same edge drives the diode to 4.10 A and 16.9 W, and the run warns for diode current and for diode power. The reset button, closed onto the capacitor with the stage removed and DTR held at 0, reaches 49.7 A. That spike is not a rating warning, because a switch has no ratings.

The host VBUS is exactly 5.0 V. The rail has no bulk capacitance, so a load step moves `v5` at once, by the step times 0.21 ohm. D1, the TX pin, is left unconnected. The chip's TX bytes are already in `traces/serial.csv`.

Left out on purpose are the 1 Mohm resistor across the resonator, the 16U2 and its crystal and LEDs, the op-amp, the ICSP header, the fuse's trip, the barrel jack, VIN, the 5 V regulator, and the power-path comparator. Rail capacitors PC2, C2, and C6 are out. C3 sits across the ideal 3.3 V source, so it is out too.

## A button on the board

`examples/atmega-board-button` is the fragment with `firmware/sketch/button.hex` and two parts added after it. `sw1` joins `d2` to `v5`. It starts open, closes at 5 ms, and opens at 15 ms. `r1` is 10 kohm from `d2` to `gnd`. The sketch reads pin 2 with the pull-up off and drives the pin-13 LED high only while that read is high. The run stops at 20 ms. The grid is 1 ms, and the run writes 21 rows. On a release run, PD2 rises at the close and falls at the open. At these switch times, PB5 rises 137 cycles after the close and falls 85 cycles after the open. A cycle is 62.5 ns, so those lags are 8.5625 us and 5.3125 us. The loop is 103 cycles. Other switch times give 61 to 262 cycles, because the lag follows that loop and the timer interrupt can land in the window. `crates/cli/tests/board.rs` pins the 137 and 85 cycle lags recorded at these switch times. The viewer draws `sw1` as `tactile-switch-6mm` and `r1` as `resistor-axial`. The fragment parts keep their ids.

## Later

A regulator part can replace `reg_3v3` and keep the net name `v3v3`, with the 1 uF output capacitor beside it. A `vin` net and the 5 V regulator from `vin` to `v5` add barrel-jack power. The automatic switch between USB and the jack waits on an op-amp and a `pmos` kind. Until then a barrel-powered board is a second fragment, with that regulator in place of `usb_vbus`, `usb_fuse`, and `usb_switch`. The pin-13 follower waits on the same op-amp. It would sit between `d13` and a new net, and `r_l` would move to that net. `l13` and `led_l` keep their names. The bootloader is not part of this fragment.
