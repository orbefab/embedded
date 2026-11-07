# orbe: core

orbe simulates a circuit and the firmware on its microcontroller. You describe the circuit in a scene file, build the firmware yourself, then drive the simulation from the shell. You press buttons, step time, and read pins and LEDs. A test is a file of the same commands with `expect` lines, and `orbe test` runs it.

This text comes from the `orbe` you have installed, so it matches it. `orbe <command> --help` shows one command's usage.

## The loop

1. Make a project folder and write `scene.jsonc` in it.
2. Build the firmware and name the built file in the scene. For an Arduino board, run `orbe skills get arduino` first.
3. Run `orbe check scene.jsonc`. It prints every error with its file, line, column and a hint. A missing part is `part_missing`, and its hint is the `orbe add` command to run. Run it as printed, then check again.
4. Run `orbe open` in the project folder. It opens `scene.jsonc` there, or the scene you name, as `orbe open path/to/scene.jsonc`. It checks the scene, starts a session at t = 0, paused, and lists the parts.
5. Drive it with `orbe step`, `orbe press`, `orbe release`, `orbe get`, `orbe expect` and `orbe snapshot`.
6. To fix something, edit the scene or the firmware, rebuild, and run `orbe open` again. It restarts the session at t = 0.
7. Run `orbe close` when the live work is done. A `no_session` error from it means nothing was open.
8. Write what worked as `tests/<name>.test` and run `orbe test`. It leaves no session to close.

## Worked example

A button on D2 lights an LED on D13 of a Nano. The project folder holds:

```
scene.jsonc
button/button.ino        the sketch, from `orbe skills get arduino`
build/button.ino.hex     the built firmware
tests/button.test
```

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
step --until @led1 on --timeout 50ms
expect @nano.D13 high
release @btn1
step 50ms
expect @led1 off
expect no diagnostics
```

The session, with what each command prints. A line `...` stands for lines left out here.

```console
$ orbe check scene.jsonc
scene.jsonc:4:23: error[part_missing]: part `nano-board` is not in this project's parts.lock
  hint: run `orbe add led-5mm nano-board resistor-axial tactile-switch-6mm`
...
4 errors in scene.jsonc
$ orbe add led-5mm nano-board resistor-axial tactile-switch-6mm
...
wrote parts/nano-board 0.1.0
...
wrote parts.lock
$ orbe check scene.jsonc
scene.jsonc: ok, 18 engine parts, 31 nets, ground nano__GND
$ orbe open
opened scene.jsonc at t=0: nano, btn1, r2, r1, led1 (18 engine parts, 31 nets)
  warning[net.floating] #net.nano__AREF has no DC path to ground
$ orbe step 100ms
t=100ms (from 0s)
$ orbe expect @led1 off
passed @led1 off (led1 off, i=1.341311570535588e-13 A)
$ orbe press @btn1
pressed btn1 at t=100ms; reads show it after t=100ms
$ orbe step --until @led1 on --timeout 50ms
t=101ms (from 100ms), @led1 on by 101ms
  100ms nano.D2 high
  100.0040625ms nano.D13 high
  by 101ms led1 on
$ orbe get @nano.D13
nano.D13 high 4.621848245658706 V
$ orbe get @led1
led1 on i=0.012246558123229083 A v=1.9276054581242548 V p=0.02360653228157231 W
$ orbe expect @led1.i '>' 5mA
passed @led1.i > 5mA (led1.i = 0.012246558123229083 A)
$ orbe snapshot
t=101ms  scene=scene.jsonc
nano  nano-board          5V=high  D13=high  D2=high  GND=0.0V
btn1  tactile-switch-6mm  i=-0.0005009040686676514 A
r2    resistor-axial      i=0.0004999949909593132 A
r1    resistor-axial      i=0.012246558125156596 A
led1  led-5mm             on i=0.012246558123229083 A
warning[net.floating] #net.nano__AREF has no DC path to ground
$ orbe close
closed at t=101ms
$ orbe test
PASS tests/button.test
1 passed, 0 failed
```

The `net.floating` warning on `nano__AREF` is normal for the Nano. Nothing drives AREF.

With D13 wired straight to `led1.anode` and no `r1`, the LED draws 105 mA. `step` lists `warning[rating.current]` and `warning[rating.power]` for `led1`, and `expect no diagnostics` fails. The LED holds D13 at 2.4 V, so D13 reads `low` while the LED is on. Give each LED a series resistor, such as 220 ohm from a 5 V pin.

## Scenes

A scene is JSONC: JSON plus `//` comments and trailing commas, nothing more. Its keys:

- `format`: always `1`.
- `parts`: each instance by name. `type` is a catalog id (the list below) or a `./` path to a part folder of your own. The other keys are the part's params. `"not_simulated": "<why>"` keeps a part in the scene for drawing but out of the simulation. Nothing may be wired to it. A wire to one of its pins is a `not_simulated` error that gives `<why>`.
- `wires`: pairs of pins, `["nano.D2", "btn1.a"]`. orbe joins wired pins into nets. Two wires to one pin are fine. A third item, `{ "kind": "jumper" }`, `{ "kind": "trace" }` or `{ "kind": "contact" }`, only says how it is drawn. `jumper` is the default.
- `nets` (optional): names a net by one of its pins, `"GND": "nano.GND"`. Other nets take the name of their smallest pin, with `.` written `__`, as `nano__GND`.
- `layout` (optional): `"btn1": { "at": [x, y, z], "rotate": [rx, ry, rz] }` in mm and degrees, for drawing only.
- `sim` (optional): `{ "max_step": "1ms", "seed": 1 }`. `max_step` is the engine's largest step, 10 ms by default.

A part name starts with a letter, takes letters, digits and `_`, and may not contain `__` or end in `_`. `i`, `v`, `p`, `pin`, `logic` and `net` are reserved. With two boards in a scene, wire their GND pins together, so both grounds are one net.

Param values:

- A number with an optional SI prefix and unit, as a string or a number, such as `"10k"`, `"4.7 kohm"`, `"220"`, `220`, `"100n"`, `"1ms"`. Prefixes: p, n, u, m, k, M.
- A choice by its name, as `"color": "red"`.
- A file by its path, relative to the file that sets it, as `"firmware": "build/button.ino.hex"`.

## Parts

`orbe add <id>…` copies parts from the catalog built into this `orbe` into `parts/<id>/` and records them in `parts.lock`. It also copies the parts a part is built from. Commit `parts/` and `parts.lock` with the scene. Don't edit a copied part. A copy that differs, even in spacing, is a `part_modified` error. A board built from other parts, such as `nano-board`, is the exception. An edit to it checks, with a note. To change a part, copy it to a folder of your own and name it by its `./` path.

<!-- catalog parts -->

## Refs

A ref names something in the open scene. It starts with `@`, which a shell leaves alone.

- `@led1`: a part. `get` prints its reading, such as `on` or `off`, and its current `i`, voltage `v` and power `p`.
- `@nano.D13`, `@led1.anode`: a pin. `get` prints its logic level, `high` or `low`, when a chip pin is on its net, and its voltage.
- `@led1.i`: a quantity of a part: `i`, `v`, `p`, and for some parts `ib`, `ic`, `id`, `iin`, `iout`. An unknown one gets an error that lists what the part reports.
- `@nano.mcu.PB5`, `@nano.led_l`: a part inside a part, by its name in that part's file. The Nano's chip is `mcu`, and `led_l` is its L LED, which D13 drives.

`press @btn1` and `release @btn1` close and open a switch.

## Conditions

`expect` and `step --until` take the same conditions:

- `@led1 on`, `@led1 off`: a part's `on` reading.
- `@nano.D13 high`, `@nano.D13 low`: a pin's logic level.
- `@led1.i > 5mA`, `@nano.D2 < 1V`: a quantity or a pin's voltage, with `<`, `<=`, `>` or `>=`, and a value in A, V or W with an optional prefix (p, n, u, m, k, M). A value as `get` prints it, written without the space, such as `2.27e-5V`, parses too.
- `serial contains "text"`: all the serial text the chip has sent since `open`, line endings included. Text printed earlier still matches. `snapshot` shows the last 10 lines of it, joined with `\n`.
- `no diagnostics`: no runtime warning has come up, such as a current over a part's rating. A chip warning can show up to one engine step (10 ms) late, so step that far past its cause first.

In a shell, quote `<` and `>`, or the shell takes them as redirects, as in `orbe expect @led1.i '>' 5mA`. A test file needs no quotes for them.

## Time

- The session is paused between commands. Time moves only inside `step`, or while `play` runs (see "Watching in 3D").
- `step <duration>` advances by that much and lists what changed, such as pin edges with their exact times, LED changes and diagnostics. It lists at most 20, then `and N more`, so serial edges on D1 can hide the rest. `get` and `snapshot` show the state after it. Durations take ps, ns, us, ms or s, such as `100ms`, `1.5ms`, `20us`.
- A read at time t, by `get`, `expect` or `snapshot`, shows the circuit just before anything that happens at t. So `press` at t shows from t + 1 ps on, and its reply says "reads show it after t". Step after an input before you read its effect.
- An input lands at the session's time or later, and its reply gives the time.
- `step --until <condition> [--timeout <duration>]` first reads the condition now. If it holds, it takes no time. Otherwise it steps until it holds, up to the timeout, 10s by default. A pin condition stops 1 ps after the exact edge, and part readings such as `@led1 on` already show the edge there, with no extra `step`. Other conditions are read every 1 ms, so the reply says `by 101ms`. A timeout is the error `until_timeout`.
- Serial can reach orbe up to one engine step (10 ms) after the chip sends it. Step a little past a print before you expect it.
- A session stops at 2 hours of simulated time. Reads still work there, and `press`, `release` and `step` fail with `ended`. A `step` that would pass the stop fails with `past_stop`. Run `orbe open` to start again.

A switch with `bounce` set bounces up to `max_bounces` times within that window after each `press` and `release`, and an input on it lands after the window of the one before. With this one-line change to the worked example's `scene.jsonc`,

```diff scene.jsonc
-    "btn1": { "type": "tactile-switch-6mm" },
+    "btn1": { "type": "tactile-switch-6mm", "bounce": "2ms" },
```

a `release` and a `press` with no `step` between land 2 ms apart, and `step --until @led1 on` stops at the first contact, which opens again inside the window:

```console
$ orbe open
...
$ orbe step 100ms
t=100ms (from 0s)
$ orbe press @btn1
pressed btn1 at t=100ms; reads show it after t=100ms
$ orbe step --until @led1 on
t=101ms (from 100ms), @led1 on by 101ms
...
$ orbe release @btn1
released btn1 at t=102.000002ms; reads show it after t=102.000002ms
$ orbe press @btn1
pressed btn1 at t=104.000004ms; reads show it after t=104.000004ms
$ orbe step 6ms
t=107ms (from 101ms)
  101.028757825ms nano.D2 low
...
$ orbe expect @led1 on
passed @led1 on (led1 on, i=0.012246558123229083 A)
$ orbe close
closed at t=107ms
```

To read a bouncing switch settled, step past the time in the last input's reply plus the window. Here that is 104.000004ms + 2ms, so `step 6ms` from 101ms.

## Test files

A test file is `tests/<name>.test` in the project folder. Each line is a live command without `orbe`. `--json`, `--session`, `--help` or `-h` on a line is a usage error, except as the text after `serial contains`. Live, `--help` anywhere prints help, so `orbe expect serial contains --help` prints help, while the same test line looks for that text. `#` starts a comment. `"..."` holds spaces, and inside it only `\"` and `\\` escape, so a test cannot name a line ending. To tell lines apart, print text that only one line has, such as `led=on`. Paths start from the project folder, and `open` alone opens `scene.jsonc`.

`orbe test` runs every `tests/*.test` of the project in the current folder, each in a fresh session, and stops a file at its first failed line. `orbe test <file or folder>…` runs those. `-v` prints every command with its reply. A failure prints the line, the expected and the actual value. With the first `expect` in `tests/button.test` changed to `expect @led1 on`:

```
FAIL tests/button.test:4: expect @led1 on
  error[expect_failed]: expected @led1 on
    expected: @led1 on
    actual:   led1 off, i=1.341311570535588e-13 A
0 passed, 1 failed
```

A test needs no background process, so it runs the same in CI. Each file's session lives inside `orbe test` and ends with it. A session that `orbe open` started keeps running, untouched. `orbe test` takes no `--session` and ignores `ORBE_SESSION`.

## Output and exit codes

Every command prints text. With `--json` it prints one JSON object instead, `{"ok": true, "data": {...}}` or `{"ok": false, "error": {"code", "message", "hint"}}`, where `hint` can be missing or empty. A failed `orbe check scene.jsonc --json` is `scene_invalid`, with each finding, its code and hint in `data.diagnostics`. Live commands add `session`, `t_ps` (picoseconds) and `state`: `paused`, `playing`, `stopped`, `faulted` or `closed`.

Exit codes: 0 ok, 1 a failed check, `expect` or command, 2 a usage error, 3 a file or system error. `orbe test` exits with the worst file's code.

## Sessions

- `orbe open` starts a background process that holds the session, and every later command talks to it. The process ends on `orbe close`, or after 1 hour with no command while it is not playing and no page watches it.
- A session has a name. By default it comes from the project folder, the nearest one up with `scene.jsonc` or `parts.lock`, so each project gets its own, and its `tests/` folder shares it. Outside a project it comes from the git repository's root folder, else the current folder. `--session <name>` after the command, as `orbe open --session bench`, or `ORBE_SESSION=<name>`, picks another, so two sessions can run side by side. Live paths start from the shell's current folder, so run live commands from the project folder.
- `orbe close --all` closes every session of this `orbe` and cleans up after crashed ones.
- A command sent while a `step` runs waits for it. Ctrl-C during a long `step` stops it, and the session stays paused at the time it reached.

## Watching in 3D

A person can watch a live session in 3D and press its buttons. `orbe open --headed` opens the scene, then a page in the default browser. `orbe show` opens the page on a session that already runs. Each prints the page's address, which only this machine can reach, and `--no-browser` prints it without opening a browser. Keep the address to yourself, since it holds the session's key.

- Several pages can watch one session. Closing a page leaves the session running, and `orbe close` ends every page.
- A click on a button in the page is `press`, and letting go is `release`. They land at the session's time, as yours do, so read the session with `get` or `snapshot` as always.
- `orbe play` runs time at real speed and returns at once, and `state` is `playing` meanwhile. Time runs until `orbe pause`, a `step` that runs, the 2-hour stop or a fault. A refused `step`, such as `past_stop` or an `unknown_ref` in `--until`, leaves it running. The page's Play button runs the same `play`.
- `play` reports nothing as time runs. The next `step` lists every change since the `step` before it, play included, such as the pin edges, LED changes and diagnostics a press from the page caused, so its reply reads like `t=12.001s (from 2s)`. When play ran into the 2-hour stop or a fault, the `step` that is refused lists them. `pause` lists nothing, and `get` and `snapshot` read the state at any time.
- A test file refuses `play` and `pause` with `usage`. Step time instead.
- `show` failing with "this orbe was built without the viewer page" means this `orbe` has no page in it. Tell the user, since the fix is a rebuild of `orbe`. When `open --headed` fails that way, the scene is open anyway, so you can work headless or `orbe close` it.

## Errors

An error prints `error[<code>]: <message>`, and most add a `hint:` line. Do what the hint says. The common ones:

| Code | What to do |
| --- | --- |
| `usage` | The command line is wrong. The message says how. |
| `part_missing` | Run the `orbe add` command in the hint. |
| `unknown_part` | `orbe add` knows no part by that id. Pick one from the parts list above. |
| `busy` | Another `orbe add` is at work in this project. Wait for it, then run yours again. |
| `firmware_missing` | Build the firmware, or fix the path. It starts from the scene's folder. |
| `not_simulated` | A wire reaches a pin the engine does not simulate, or a part marked `not_simulated`. Wire another pin, or remove the wire. |
| `scene.unknown_pin`, `scene.param`, `scene.syntax` | A typo or a wrong value at that line and column. The hint names what is valid. |
| `part_modified` | A copied part was edited. Run `orbe add <id> --overwrite` to restore it, or copy it to a folder of your own. |
| `no_session` | No session runs under this name. Run the `orbe open` in the hint. |
| `session_crashed` | The session's process died. Run `orbe open` again. |
| `unknown_ref`, `no_reading` | The ref names nothing, or nothing to read. The hint lists what exists. |
| `not_settable` | `press` and `release` work only on a switch, by its part name. |
| `expect_failed`, `until_timeout` | The condition did not hold. Compare `actual`, then fix the circuit or the firmware, or step longer. |
| `ended`, `past_stop` | The session reached its 2-hour stop. Run `orbe open`. |
| `cancelled` | Ctrl-C stopped a `step`. The session stays at the time it reached. Step again. |
| `faulted` | The engine stopped the session. The message names why. Fix it and run `orbe open`. |

## What is not simulated

- The Nano's 3V3, VIN, A6 and A7 pins. A wire to one is a `not_simulated` error.
- USB, and serial into the chip. The chip's serial out (TX, D1) is read as text.
- Light, temperature, a switch's force and travel, and part tolerances. An LED's `on` is its current over a threshold.
- Any part the catalog above does not list.
- orbe never builds firmware. Build it with your toolchain and name the output file in the scene.
