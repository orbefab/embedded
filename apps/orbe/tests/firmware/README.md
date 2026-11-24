Firmware the `orbe test` tests load in place of the first target's `button.ino.hex`. Each `<name>.hex` is the `<name>.ino.hex` that `arduino-cli compile --fqbn arduino:avr:nano --output-dir build <name>` writes for the `<name>.ino` beside it.

- `pulse` sets D13 high for 50 us at 10 ms, then leaves it low.
- `flood` prints `ready`, then a 36-character line forever, at 115200 baud.
- `utf8` prints `café ok` at 115200 baud.
- `amp` prints `I=0.25 A` at 115200 baud.
- `split` prints forty `é`, then `|bad\xff\xfe|`, then the first two bytes of `€` at 115200 baud. After 20 ms it prints the third byte and ` done`.
- `seg_dim` shows 8 on segments a to g, on D2 to D8, and runs `analogWrite(10, 200)` on the common cathode at D10.
- `seg_mux` counts 0 to 9 on D2 to D8, each digit for 20 frames of 10 ms, with the common cathode on D10 low for 2.5 ms of each frame.
- `seg_count` shows each digit 0 to 9 on D2 to D8 for 100 ms with dp on D9 off, then 100 ms with dp on, then dp alone for 100 ms. D11 low drives each segment low, for a common-anode display.
