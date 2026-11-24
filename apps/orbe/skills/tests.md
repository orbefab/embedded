# orbe: tests

Load this when a test does more than the example in `orbe skills get core`.

A test is `tests/<name>.test`. Each line is a live command without `orbe`. `#` starts a comment. `"..."` holds spaces. Inside quotes only `\"` and `\\` escape, so a test cannot name a line ending. To tell lines apart, print text only one line has, such as `led=on`. `open` alone opens `scene.jsonc`.

`orbe test` runs every `tests/*.test`, each in a fresh session, and stops a file at its first failed line. `orbe test <file or folder>…` runs only those. `orbe test -v` prints each command and its reply. The file's session ends with `orbe test`. A session from `orbe open` keeps running, untouched. `orbe test` takes no `--session` and ignores `ORBE_SESSION`.

`--json`, `--session`, `--help` or `-h` on a line is `usage`, except as the text after `serial contains`. Live, `--help` anywhere prints help, so `orbe expect serial contains --help` prints help, while the same test line looks for that text. A test file refuses `play` and `pause` with `usage`.

`expect` and `step --until` take the same conditions. Quote `<`, `>` and a colour in a shell.

- `@led1 lit`, `@led1 dim`, `@led1 off`. Load `orbe skills get ratings` for the current each state needs.
- `@relay closed`, `@relay open`. A relay contact. The coil current is that part's `i`, and on the 1-channel module it is `@relay.k.i`.
- `@disp.digit 8` on a 7-segment display, or `blank` when none of a to g glows. A dim segment counts. Another pattern names the glowing segments, and the decimal point stays out of the digit. A new digit reads within 1 ms. `orbe get @disp` and `orbe get @disp.digit` print the same word.
- `@nano.D13 high`, `@nano.D13 low`.
- `@led1 rgb(255,0,0)`, an addressable LED's latched colour. `#` starts a comment, so a colour is never hex. In a shell, `orbe expect @led1 'rgb(255,0,0)'`.
- `@oled.display.screen lit` or `off`, whether a screen lights any pixel. For a 128 x 64 I2C OLED, use the part `oled-ssd1306-128x64`, as the Block `nano-oled` does. `@oled.display.screen shows tests/hello.screen` compares each row with that file, from the scene's folder. `orbe get @oled` and `orbe get @oled.display.screen` print the rows to save, `#` lit and `.` dark.
- `@lcd.text` on a 16 x 2 I2C LCD (`lcd1602-i2c`, Block `nano-lcd1602-i2c`): `orbe get @lcd` and `orbe get @lcd.text` print the two visible 16-character lines (blank when the display or backlight is off; custom CGRAM characters read as `□`). Expect `@lcd.text contains "Hello"`, `@lcd.text line 0 "Hello"`, `@lcd.text line 1 "World"`, `@lcd.text blank`, or `@lcd.backlight lit` / `off`.
- `@led1.i > 5mA`, `@nano.D2 < 1V`, `@bz.hz > 400Hz`, `@servo.angle > 60deg`, with `<`, `<=`, `>` or `>=`. `@bz.hz` is 0 when the buzzer is quiet. In a shell, `orbe expect @led1.i '>' 5mA`. A test line accepts a quoted comparison the same way a shell does, as `'>'` or `"<="`. `orbe get @servo` and `orbe get @servo.angle` read a servo's angle in `deg`.
- `serial contains "text"`, all the serial text since `open`. `snapshot` shows the last 10 lines, joined with `\n`.
- `no diagnostics`, or `diagnostics contains "text"`. A chip warning can show up to one engine step (`sim.max_step`, 10 ms by default) late, so step that far past its cause first.

A failure prints the line, what was expected and what it saw, then `0 passed, 1 failed`.

```bash
# run one file and show each reply
orbe test -v tests/button.test
```
