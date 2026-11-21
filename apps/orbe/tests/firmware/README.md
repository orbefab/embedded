Firmware the `orbe test` tests load in place of the first target's `button.ino.hex`. Each `<name>.hex` is the `<name>.ino.hex` that `arduino-cli compile --fqbn arduino:avr:nano --output-dir build <name>` writes for the `<name>.ino` beside it.

- `pulse` sets D13 high for 50 us at 10 ms, then leaves it low.
- `flood` prints `ready`, then a 36-character line forever, at 115200 baud.
- `utf8` prints `café ok` at 115200 baud.
- `amp` prints `I=0.25 A` at 115200 baud.
- `split` prints forty `é`, then `|bad\xff\xfe|`, then the first two bytes of `€` at 115200 baud. After 20 ms it prints the third byte and ` done`.
