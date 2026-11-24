# orbe

orbe simulates a circuit and the firmware on its microcontroller. You describe the circuit in a scene, build the firmware, then drive the simulation from the shell. This text is from the `orbe` you have installed. `orbe <command> --help` shows one command. `orbe blocks`, `orbe search <words>` and `orbe part <id>` show Blocks and parts. Do not paste their output into a scene by hand, and do not search the disk or the `orbe` binary for a part.

## Rules

- Run one `orbe skills get <name>` per command. Do not chain it after another command.
- For a part, run `orbe part <id>`. It prints the pins, params, ratings and what it does not simulate.
- When orbe can't show or check something the user asked for, stop and say what it can't show and why. Don't fake a test, such as one that checks only text the firmware prints about itself.
- Close every session with `orbe close` before you hand back, and leave no background loop running. When the user watches in 3D, leave the session open and tell them to run `orbe close`.

## The loop

```bash
# list Blocks, or find a part or a Block
orbe blocks
orbe search <words>
# a close Block, in an empty folder, when the user did not ask to build from parts
orbe add <block>
orbe check scene.jsonc
orbe test
```

1. Start from a Block only when one is close to the circuit and the user did not ask to build it from parts. `orbe add <block>` in an empty folder writes a scene, firmware and a test that already work. Check and test, then change it. Tell the user which Block you started from and what you changed.
2. Otherwise find each part with `orbe search <words>`, read it with `orbe part <id>`, and download it with `orbe add <id>…`. A load such as an LED strip or a heater is `resistor-sqp15`. A separate supply is `barrel-jack-dc21` with its `voltage`.
3. Write `scene.jsonc`. To seat a part on a breadboard, load `orbe skills get breadboards` first.
4. Run `orbe check scene.jsonc` and `orbe lint scene.jsonc` after every edit. With no path, both read `scene.jsonc`. Do what each hint says. A `part_missing` hint is the `orbe add` to run. `orbe nets` lists each net and its pins, and marks `(floating)` on a net with no DC path to ground.
5. Build the firmware and name the file in the scene. For a Nano or an Uno, load `orbe skills get arduino` first.
6. `orbe open` starts a session at t = 0, paused. Drive it with `step`, `press`, `release`, `set`, `get`, `expect` and `snapshot`. After a firmware fix, rebuild and `orbe open` again. Save what works as `tests/<name>.test` and run `orbe test`.
7. When the user wants to watch, run `orbe open --headed` and `orbe play`. Load `orbe skills get page` for the page.
8. `orbe close` when the live work is done. A `no_session` error means nothing was open. `orbe test` leaves no session to close.

## Worked example

A button on D2 lights an LED on D13 of a Nano. The folder holds `scene.jsonc`, the sketch from `orbe skills get arduino`, `build/button.ino.hex` and `tests/button.test`.

```jsonc scene.jsonc
{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "btn1": { "type": "tactile-switch-6mm" },
    "r2": { "type": "resistor-axial", "resistance": "10k" }, // pull-down, so D2 reads low while open
    "r1": { "type": "resistor-axial", "resistance": "220" }, // in series, so led1 stays under its rating
    "led1": { "type": "led-5mm" },
  },
  "wires": [
    ["nano.D2", "btn1.a"],
    ["btn1.b", "nano.5V"],
    ["nano.D2", "r2.a"],
    ["r2.b", "nano.GND"],
    ["nano.D13", "r1.a"],
    ["r1.b", "led1.anode"],
    ["led1.cathode", "nano.GND"],
  ],
}
```

```text tests/button.test
# Pressing the button lights the LED, and releasing it turns the LED off.
open
step 100ms
expect @led1 off
press @btn1
step --until @led1 lit --timeout 50ms
expect @nano.D13 high
release @btn1
step 50ms
expect @led1 off
expect no diagnostics
```

```console
$ orbe check scene.jsonc
scene.jsonc:4:23: error[part_missing]: part `nano-board` is not in parts/
  hint: run `orbe add led-5mm nano-board resistor-axial tactile-switch-6mm`
...
$ orbe add led-5mm nano-board resistor-axial tactile-switch-6mm
...
wrote parts/led-5mm 0.3.0
...
$ orbe check scene.jsonc
scene.jsonc: ok, 5 scene parts, expanding to 22 engine parts, 36 nets, ground GND
$ orbe lint scene.jsonc
...
1 warning in scene.jsonc
$ orbe open
opened scene.jsonc at t=0: nano, btn1, r2, r1, led1 (5 scene parts, expanding to 22 engine parts, 36 nets)
$ orbe step 100ms
t=100ms (from 0s)
...
$ orbe press @btn1
pressed btn1 at t=100ms; reads show it after t=100ms
$ orbe step --until @led1 lit --timeout 50ms
t=104ms (from 100ms), @led1 lit by 104ms
...
$ orbe close
closed at t=104ms
$ orbe test
PASS tests/button.test
1 passed, 0 failed
```

The lint warning is `scene.unplaced`: there is no `layout`, so the 3D page puts the parts in a row. This example is not a 3D scene.

A ref starts with `@`. `@led1` is a part, and `get` prints `lit`, `dim` or `off`, plus `i`, `v` and `p`. `@nano.D13` is a pin, and `get` prints `high` or `low` only when a chip pin is on that net, plus the voltage. `@led1.i` is one quantity. `@nano.mcu.PB5` names a part inside a part; on the Nano and the Uno the chip is `mcu`. Quote `<` and `>` in a shell, as `orbe expect @led1.i '>' 5mA`. Time moves only in `step`, or while `play` runs. A read shows the circuit just before anything at that time, so step after an input before you read its effect. `step --until` waits 10s unless you pass `--timeout`.

## Troubleshooting

An error prints `error[<code>]: <message>` and usually a `hint:`. Do what the hint says. Exit codes: 0 ok, 1 a failed check, `expect` or command, 2 a usage error, 3 a file or system error. `--json` puts the same code in the envelope. Load `orbe skills get errors` for every other code.

| You see | What to do |
| --- | --- |
| `error[part_missing]` | Run the `orbe add` in the hint. |
| `error[firmware_missing]` | Build the firmware, or fix the path. It starts from the scene's folder. |
| `error[unknown_ref]` | The hint lists what exists. |
| `error[expect_failed]` | Compare `actual`. Fix the circuit or the firmware. |
| `error[until_timeout]` | The condition did not hold in time. Compare `actual`, or step longer. |
| `error[no_session]` | Nothing is open under this name. Run the `orbe open` in the hint. |
| `warning[rating.current]`, `warning[rating.power]` or `warning[rating.voltage]` | A part or a pin went past its rating. Load `orbe skills get ratings`. |
| `warning[chip.floating_input]` | An input nothing drives. Add a pull-down, or use `INPUT_PULLUP` to GND. Load `orbe skills get arduino`. |
| `error[scene.layout]` | A bad seating. Load `orbe skills get breadboards`. |
| `warning[scene.unplaced]` | The 3D page has no place for a part. Fine unless someone is watching. Load `orbe skills get breadboards` to seat it. |

## Load more when you need it

`orbe skills list` names every topic. Load one only when you need it.

| Load | When you need |
| --- | --- |
| `orbe skills get arduino` | Building Nano or Uno firmware, or a chip warning |
| `orbe skills get esp32` | Building ESP32-S3 firmware for an `esp32s3-devkitc-1` |
| `orbe skills get scenes` | A scene key, a part name, a param or `world` |
| `orbe skills get breadboards` | Seating a part, a hole, or a breadboard error |
| `orbe skills get parts` | The copy in `parts/`, a version, a Block, `buy`, an addressable LED, or a 5 V board wired to a 3.3 V module |
| `orbe skills get sessions-and-time` | When a step or a set lands, a session name, bounce, or the 2-hour stop |
| `orbe skills get tests` | A test file beyond the example above, or a screen |
| `orbe skills get ratings` | A warning names a rating, or an LED is lit, dim or off |
| `orbe skills get errors` | An error code not in the table above |
| `orbe skills get page` | The 3D page, or turning a knob |
| `orbe skills get not-simulated` | A pin or a feature does nothing, or `error[not_simulated]` |
