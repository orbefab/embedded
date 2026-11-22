# orbe: core

orbe simulates a circuit and the firmware on its microcontroller. You describe the circuit in a scene file, build the firmware yourself, then drive the simulation from the shell. You press buttons, step time, and read pins and LEDs. A test is a file of the same commands with `expect` lines, and `orbe test` runs it.

This text comes from the `orbe` you have installed, so it matches it. `orbe <command> --help` shows one command's usage.

## Rules

- Fetch one skill per command. Run `orbe skills get arduino` on its own, not chained after another. `orbe skills list` names every topic: `parts` for how a project keeps its parts, `reference` for the full tables and details.
- For a part, read `orbe part <id>`. It prints the part's pins, params, ratings and what it does not simulate. Never search the disk or the `orbe` binary for a part or its model.
- When orbe can't show or check something the user asked for, stop and tell the user what orbe can't show and why. Don't fake a test, such as one that checks only text the firmware prints about itself.
- Close every session you open with `orbe close` before you hand back, and leave no background loop running. When the user watches a session in 3D, leave it open and tell them to run `orbe close` when they are done.

## The loop

1. `orbe blocks` lists the Blocks, and `orbe search <words>` finds one. When a Block is close to the circuit, run `orbe add <block>` in an empty folder. It writes a scene, firmware and a test that already work. Run `orbe check scene.jsonc` and `orbe test`, then change it.
   - `nano-button-led`: a button on D2 lights a red LED on D13 of an Arduino Nano, and releasing it turns the LED off.
   - `uno-button-led`: a button on D2 lights a red LED on D13 of an Arduino Uno, and the board's own L LED with it.
   - `nano-led-fade`: an LED on D9 of an Arduino Nano fades up and down by PWM.
   - `nano-light-comparator`: an LDR and a trimmer set a threshold. An LM358 lights a red LED in the dark, and a Nano on D2 prints dark or light.
2. Otherwise find each part with `orbe search <words>`, read it with `orbe part <id>`, and download it into the project with `orbe add <id>…`.
3. Write `scene.jsonc` in the project folder. Seat each part on a breadboard with `on`.
4. Run `orbe check scene.jsonc`, then `orbe lint scene.jsonc`, after every edit. With no path, both read `scene.jsonc`. Do what each hint says. A `part_missing` hint is the `orbe add` command to run. `orbe nets` lists each net and its pins, and marks `(floating)` on a net with no DC path to ground.
5. Build the firmware and name the built file in the scene. For an Arduino board, run `orbe skills get arduino` first.
6. Run `orbe open`, which starts a session at t = 0, paused. Drive it with `step`, `press`, `release`, `set`, `get`, `expect` and `snapshot`. After a fix, rebuild and run `orbe open` again. Write what works as `tests/<name>.test` and run `orbe test`.
7. When the user wants to watch, run `orbe open --headed` and `orbe play`.
8. Run `orbe close` when the live work is done. A `no_session` error from it means nothing was open. `orbe test` leaves no session to close.

## Worked example

A button on D2 lights an LED on D13 of a Nano. The project folder holds `scene.jsonc`, the sketch `button/button.ino` from `orbe skills get arduino`, its build `build/button.ino.hex`, and `tests/button.test`.

```jsonc scene.jsonc
{
  "format": 1,
  "parts": {
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "btn1": { "type": "tactile-switch-6mm" },
    "r2": { "type": "resistor-axial", "resistance": "10k" }, // pull-down, so D2 reads low while open
    "r1": { "type": "resistor-axial", "resistance": "220" }, // in series, so led1 stays under its rating
    "led1": { "type": "led-5mm", "color": "red" },
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

The session, with what each command prints. A line `...` stands for lines left out here.

```console
$ orbe check scene.jsonc
scene.jsonc:4:23: error[part_missing]: part `nano-board` is not in parts/
  hint: run `orbe add led-5mm nano-board resistor-axial tactile-switch-6mm`
...
$ orbe add led-5mm nano-board resistor-axial tactile-switch-6mm
...
wrote parts/led-5mm 0.2.0
...
$ orbe check scene.jsonc
scene.jsonc: ok, 5 scene parts, expanding to 20 engine parts, 35 nets, ground GND
$ orbe lint scene.jsonc
...
1 warning in scene.jsonc
$ orbe open
opened scene.jsonc at t=0: nano, btn1, r2, r1, led1 (5 scene parts, expanding to 20 engine parts, 35 nets)
$ orbe step 100ms
t=100ms (from 0s)
...
$ orbe press @btn1
pressed btn1 at t=100ms; reads show it after t=100ms
$ orbe step --until @led1 lit --timeout 50ms
t=104ms (from 100ms), @led1 lit by 104ms
  100ms nano.D2 high
  100.0040625ms nano.D13 high
...
$ orbe get @nano.D13
nano.D13 high 4.400713544697175 V
$ orbe expect @led1.i '>' 5mA
passed @led1.i > 5mA (led1.i = 11.292504375073351 mA)
$ orbe close
closed at t=104ms
$ orbe test
PASS tests/button.test
1 passed, 0 failed
```

The lint warning says the scene has no `layout`, so the 3D page places the parts in a row for drawing only. This example is not a 3D scene, so the parts stay unplaced.

## Scenes

A scene is JSONC: JSON plus `//` comments and trailing commas. Its keys:

- `format`: always `1`.
- `parts`: each instance by name. `type` is a part id or a `./` path to a part folder of your own. The other keys are the part's params. `"not_simulated": "<why>"` keeps a part for drawing only. Nothing may be wired to it. A wire to one of its pins is a `not_simulated` error that gives `<why>`.
- `wires`: pairs of pins, `["nano.D2", "btn1.a"]`. Wired pins join into nets.
- `nets` (optional): names a net by one of its pins, `"GND": "nano.GND"`. The ground net is named `GND` when `nets` does not give that name to another net. A hole never names a net. Every other net takes the name of its smallest pin, with `.` written `__`.
- `layout` (optional): `"r1": { "on": { "a": "bb.d25" } }` seats a part in a breadboard, and `"bb": { "at": [x, y, z], "rotate": [rx, ry, rz] }` places one, in mm and degrees.
- `ground` (optional): a pin on the ground net, as `"ground": "bb.X1"`, for a scene with no board to name one. The net is still named `GND` when that name is free.
- `sim` (optional): `{ "max_step": "1ms" }`, the engine's largest step, 10 ms by default.
- `world` (optional): the room, the same for every part. Each quantity holds a number, or `{ "initial": 25, "steps": [{ "at": "2s", "to": 30 }] }`:

  | Quantity | Unit | Range | Default |
  | --- | --- | --- | --- |
  | `temperature` | degC | -40 to 125 | 25 |
  | `humidity` | %RH | 0 to 100 | 50 |
  | `light` | lx | 0 to 100000 | 300 |

A part name starts with a letter, takes letters, digits and `_`, and may not contain `__` or end in `_`. `i`, `v`, `p`, `ib`, `ic`, `id`, `iin`, `iout`, `pin`, `logic`, `usart` and `net` are taken. A param value is a number with an optional prefix (p, n, u, m, k, M) and unit, such as `"10k"`, `"100n"` or `"1ms"`, a choice by name, as `"color": "red"`, or a file path from the scene's folder, as the firmware. The part reads that file as text, such as a hex, or as bytes when its file param says `"as": "blob"`, such as an app image.

## Parts

`orbe search <words>` lists the parts whose id, title, maker, number, summary or tags hold every word, then the Blocks. `orbe part <id>` prints one part: its pins and the leads they are on, params, ratings, what it does not simulate, datasheets and files. That print is the part's electrical behaviour. `orbe add <id>…` downloads the latest part into `parts/<id>/`, and the parts it is built from into `parts/<id>/parts/`. The copy is the project's: edit it, and commit `parts/` with the scene. `orbe check`, `orbe open` and `orbe test` read that copy and never use the network. `orbe add`, `orbe search` and `orbe part` need curl. `orbe add <block>` installs a Block. The Blocks are `nano-button-led`, `uno-button-led`, `nano-led-fade` and `nano-light-comparator`.

In a part's folder, `part.jsonc` is the part: its pins, params, ratings and the engine model it names, which runs inside orbe. `<id>.glb` is its 3D shape, and `<id>.anchors.json` says where each lead sits, for drawing and seating. `orbe add` copies `part.jsonc`, the GLB and the anchors when the part has them. It does not copy `model.py` or `asset.json`. Those build the 3D shape and hold nothing electrical. `orbe skills get parts` covers the copy and versions.

## Breadboards

`breadboard-400` (30 columns) and `breadboard-830` (63 columns) are parts, and each hole is a pin, as `bb.c12`. Rows `a` to `e` sit on one side of the centre channel and `f` to `j` on the other. The five holes of a column on one side are one strip, so one net. Beside row a, rail `W` is + and `X` is -. Beside row j, `Y` is + and `Z` is -. `orbe part breadboard-400` prints every hole.

Give the board an `at`. Then seat every part with leads using `on`: name one pin and the hole its lead goes into, and orbe seats the other leads where the part's shape puts them, each in its hole's strip. That works for leads off the 2.54 mm grid too. `"ldr": { "on": { "a": "bb.c21" } }` seats the LDR's lead b in c22. `"rotate": [0, 0, 90]` turns a part a quarter turn first. Wire the rest with jumpers between holes.

`at` alone only draws a part where you put it. Its leads join no hole. When that `at` puts a leg in a breadboard hole and that leg joins nothing there, `orbe lint` warns `scene.unplaced` and names the leg. The hint seats it with `on`, or a contact wire when that would claim a hole a wire holds, and keeps a non-zero `rotate`. A leg already on that hole's net, or one further below the board than the hole is deep, does not warn. Run `orbe check` and `orbe lint` after seating: a lead in the channel, off the board or in a taken hole is an error whose hint names a placement that fits, when one does, and two pins of one part in one strip warn `scene.shorted`. `orbe skills get reference` has a whole breadboard scene.

## Driving a session

`orbe open [<scene>]` starts a session on `scene.jsonc`, or on the scene named. A ref names something in the open scene. It starts with `@`, which a shell leaves alone.

- `@led1`: a part. `get` prints its state, such as `lit`, `dim` or `off`, and its current `i`, voltage `v` and power `p`.
- `@nano.D13`, `@led1.anode`: a pin. `get` prints its level, `high` or `low`, when a chip pin is on its net, and its voltage.
- `@led1.i`: one quantity of a part. An unknown one is an error that lists what the part reports.
- `@led1.color`: the colour an addressable LED, such as the sk68xxmini-hs, last latched, as `rgb(r,g,b)` from 0 to 255, and when. `get @led1` prints it as `color=`.
- `@nano.mcu.PB5`, `@nano.led_l`: a part inside a part, by its name in that part's file. On the Nano and the Uno the chip is `mcu`, and `led_l` is the L LED, which D13 drives.

`press @btn1` and `release @btn1` close and open a switch. `orbe set light 50` changes a `world` quantity from then on. `orbe get light` reads the value in force, and `orbe get @world.light` reads the same.

`expect` and `step --until` take the same conditions:

- `@led1 lit`, `@led1 dim`, `@led1 off`: an LED's state, from its `brightness`, its average current over 20 ms. It is `lit` above 2 mA and `off` at or below 100 uA. `@led1.brightness > 1mA` compares that average.
- `@nano.D13 high`, `@nano.D13 low`: a pin's level.
- `@led1 rgb(255,0,0)`: an addressable LED's latched colour. `#` starts a comment, so a colour is never hex.
- `@led1.i > 5mA`, `@nano.D2 < 1V`: a quantity or a pin's voltage, with `<`, `<=`, `>` or `>=`. In a shell, quote `<` and `>`, as `orbe expect @led1.i '>' 5mA`.
- `serial contains "text"`: all the serial text since `open`. `snapshot` shows the last 10 lines of it, joined with `\n`.
- `no diagnostics`: no runtime warning has come up. A chip warning can show up to one engine step (`sim.max_step`, 10 ms by default) late, so step that far past its cause first.

Time moves only inside `step`, or while `play` runs. `step 100ms` advances by that much and lists what changed. A read at time t shows the circuit just before anything at t, so step after an input before you read its effect. `step --until <condition> [--timeout <duration>]` steps until the condition holds, up to the timeout, 10s by default. An LED's state follows its 20 ms average, so it changes a few ms after its pin does.

## Test files

A test is `tests/<name>.test` in the project folder. Each line is a live command without `orbe`. `#` starts a comment, `"..."` holds spaces, and `open` alone opens `scene.jsonc`. `orbe test` runs every `tests/*.test`, each in a fresh session, and stops a file at its first failed line. `-v` prints each command with its reply. Each file's session lives inside `orbe test` and ends with it. A session that `orbe open` started keeps running, untouched. `orbe test` takes no `--session` and ignores `ORBE_SESSION`.

A failure prints the line, the expected and the actual value. With the first `expect` in `tests/button.test` changed to `expect @led1 lit`:

```
FAIL tests/button.test:4: expect @led1 lit
  error[expect_failed]: expected @led1 lit at t=100ms
    expected: @led1 lit
    actual:   led1 off, i=1.2775403141980628e-13 A, brightness=1.2775403141982445e-13 A over 20ms
0 passed, 1 failed
```

## Reading results

`step` lists pin edges with their exact times, LED changes and diagnostics. It lists at most 20, for what `play` held and again for what the step did, then `and N more`, then every LED's `brightness`. A `warning[...]` line is a runtime diagnostic. `rating.current`, `rating.power` and `rating.voltage` mean a part or a chip pin went past its datasheet rating, such as an LED with no series resistor. Fix the circuit. `chip.floating_input` names an input that nothing drives.

An error prints `error[<code>]: <message>`, and most add a `hint:` line. Do what the hint says. The common ones are `part_missing`, `firmware_missing` (build it, or fix the path), `unknown_ref` (the hint lists what exists), `expect_failed` and `until_timeout` (compare `actual`, then fix the circuit or the firmware, or step longer). `orbe skills get reference` has more.

Exit codes: 0 ok, 1 a failed check, `expect` or command, 2 a usage error, 3 a file or system error.

With `--json` a command prints one JSON object, `{"ok": true, "data": {...}}` or `{"ok": false, "error": {"code", "message", "hint"}}`. A failed `orbe check scene.jsonc --json` uses the same `error.code` as `orbe open --json`: `part_missing` or `firmware_missing` when every finding has that code, and `scene_invalid` otherwise, with each finding, its code and hint in `data.diagnostics`. Live commands add `session`, `t_ps` (picoseconds) and `state`: `paused`, `playing`, `stopped`, `faulted` or `closed`.

## Watching in 3D

`orbe open --headed` opens the scene and a page in the default browser, where a person watches it in 3D and clicks its buttons. When a viewer is already connected, it prints the address and opens no browser. `orbe show` opens the page on a session that already runs, and `--no-browser` only prints its address. The address holds the session's key, so give it only to the user. A session opens paused, so run `orbe play` before you hand over the page. Time then runs at real speed until `orbe pause`, a `step` that runs, the 2-hour stop or a fault, and uses the CPU while it runs. `orbe pause`, or the next `orbe step`, lists what changed while playing, including what the user did on the page.
