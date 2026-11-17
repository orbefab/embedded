# orbe: core

orbe simulates a circuit and the firmware on its microcontroller. You describe the circuit in a scene file, build the firmware yourself, then drive the simulation from the shell. You press buttons, step time, and read pins and LEDs. A test is a file of the same commands with `expect` lines, and `orbe test` runs it.

This text comes from the `orbe` you have installed, so it matches it. `orbe <command> --help` shows one command's usage.

## The loop

1. Make a project folder and write `scene.jsonc` in it.
2. Build the firmware and name the built file in the scene. For an Arduino board, run `orbe skills get arduino` first.
3. Run `orbe check scene.jsonc`. It prints every error with its file, line, column and a hint. A missing part is `part_missing`, and its hint is the `orbe add` command to run. Run it as printed, then check again.
4. Run `orbe lint scene.jsonc`. It runs `orbe check` first, then warns of what checks but is likely wrong or unclear, such as an LED with no series resistor. Read each warning. Do what its hint says, or leave it when the scene means it. Run `orbe lint` again after every edit to the scene.
5. Run `orbe open` in the project folder. It opens `scene.jsonc` there, or the scene you name, as `orbe open path/to/scene.jsonc`. It checks the scene, starts a session at t = 0, paused, and lists the parts.
6. Drive it with `orbe step`, `orbe press`, `orbe release`, `orbe set`, `orbe get`, `orbe expect` and `orbe snapshot`.
7. To fix something, edit the scene or the firmware, rebuild, run `orbe lint`, and run `orbe open` again. It restarts the session at t = 0.
8. Run `orbe close` when the live work is done. A `no_session` error from it means nothing was open.
9. Write what worked as `tests/<name>.test` and run `orbe test`. It leaves no session to close.

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
$ orbe lint scene.jsonc
scene.jsonc:1:1: warning[scene.ground_unnamed]: the ground net is `nano__GND`, a name taken from a pin, because "nets" does not name it
  hint: add "GND": "nano.GND" to the scene's "nets", so refs and messages say GND
scene.jsonc:4:5: warning[scene.unplaced]: `nano`, `btn1`, `r2`, `r1` and `led1` have no position, so the 3D page places them in a row for drawing only
  hint: place a part whose place matters in the scene's "layout", as "nano": { "at": [x, y, z] } in mm
scene.jsonc:4:5: warning[net.floating]: #net.nano__AREF has no DC path to ground
  hint: if a part reads `nano.AREF`, wire it to a source, or through a resistor to ground; a net nothing reads can stay as it is
3 warnings in scene.jsonc
$ orbe open
opened scene.jsonc at t=0: nano, btn1, r2, r1, led1 (18 engine parts, 31 nets)
  warning[net.floating] #net.nano__AREF has no DC path to ground
$ orbe step 100ms
t=100ms (from 0s)
  brightness led1 off 1.3413115705356304e-13 A over 20ms
  brightness nano.led_l off 1.34130540301447e-13 A over 20ms
  brightness nano.led_pwr lit 0.0032425854395092503 A over 20ms
$ orbe expect @led1 off
passed @led1 off (led1 off, i=1.341311570535588e-13 A, brightness=1.3413115705356304e-13 A over 20ms)
$ orbe press @btn1
pressed btn1 at t=100ms; reads show it after t=100ms
$ orbe step --until @led1 lit --timeout 50ms
t=104ms (from 100ms), @led1 lit by 104ms
  100ms nano.D2 high
  100.0040625ms nano.D13 high
  by 103ms led1 dim, brightness=0.0018344961364796203 A over 20ms
...
  by 104ms led1 lit, brightness=0.002446824042634368 A over 20ms
...
$ orbe get @nano.D13
nano.D13 high 4.621848245658706 V
$ orbe get @led1
led1 lit i=0.012246558123229083 A v=1.9276054581242548 V p=0.02360653228157231 W brightness=0.002446824042634368 A over 20ms
$ orbe expect @led1.i '>' 5mA
passed @led1.i > 5mA (led1.i = 0.012246558123229083 A)
$ orbe snapshot
t=104ms  scene=scene.jsonc
nano  nano-board          5V=high  D13=high  D2=high  GND=0.0V
btn1  tactile-switch-6mm  i=-0.0005009040686676514 A
r2    resistor-axial      i=0.0004999949909593132 A
r1    resistor-axial      i=0.012246558125156596 A
led1  led-5mm             lit i=0.012246558123229083 A brightness=0.002446824042634368 A over 20ms
warning[net.floating] #net.nano__AREF has no DC path to ground
$ orbe close
closed at t=104ms
$ orbe test
PASS tests/button.test
1 passed, 0 failed
```

The example keeps its 3 warnings, since each is what it means: no `layout`, and the ground named after `nano.GND`. The `net.floating` warning on `nano__AREF` is normal for the Nano and the Uno. Nothing drives AREF until you wire it.

With D13 wired straight to `led1.anode` and no `r1`, the LED draws 105 mA. `step` lists `warning[rating.current]` and `warning[rating.power]` for `led1`, and `step 20ms` after it lists `warning[rating.current]` for `#nano__mcu.pin.PB5 (nano.D13)`, the chip pin behind D13, over its 40 mA rating. `expect no diagnostics` fails. The LED holds D13 at 2.4 V, so D13 reads `low` while the LED is lit. `orbe lint` warns of it before any run, with `warning[scene.led_no_resistor]` on `led1`. Give each LED a series resistor, such as 220 ohm from a 5 V pin.

The board's ATmega328P has the datasheet's absolute maximum ratings, and a run warns when a circuit breaks one. The current ratings are DC ratings, so the run checks a pin's current, and the supply's, by its RMS over the last 10.2 ms. That is five periods of the 490 Hz PWM `analogWrite` makes on D3, D9, D10 and D11, so a PWM load counts by its RMS, not its peak: `analogWrite(9, 128)` into 75 ohm peaks at 50 mA and stays under. The pulse that charges a capacitor on a pin, 0.2 A for a few nanoseconds into 100 pF, stays well under; a load held over the rating warns within those 10.2 ms:

- `rating.current` on a pin that sources or sinks over 40 mA, such as D9 into 47 ohm (69 mA), a pin driving low wired to 5V, or five LEDs with their own 220 ohm on one pin (each LED is inside its rating, the pin is not).
- `rating.current` on `VCC` or `GND` when the pins together pass over 200 mA, such as six pins at 34 mA each.
- `rating.voltage` on a pin below -0.5 V or above the chip's VCC + 0.5 V, such as 9 V through a resistor into A0.

The board's USB supply is rated at 500 mA, the most a USB port gives a high-power device. A load over that on 5V, such as 2.5 ohm from 5V to GND (2.0 A), warns `rating.current` on the supply, `#nano__usb__host.i (nano.usb)`, at 2.0 A over its 0.5 A rating. A wire from the Nano's 5V straight to GND is a short: `orbe check` and `orbe open` refuse the scene with `net.short`, naming the wire. On the Uno the same wire runs through the board's fuse and USB switch, about 0.2 ohm, and warns `rating.current` at about 24 A. Remove the wire, and feed any load through a part that limits its current.

The message names the chip pin, then the board's pins on its net, as in `#nano__mcu.pin.PB1 (nano.D9) carries 0.0434 A RMS over the last 10.2 ms, over its 0.04 A rating`, the RMS when `step` first saw it. D9 held into 47 ohm settles at 69 mA. Once a warning has lasted, its message ends with the time it has been over, as in `for 32582912000 ps`. A pin wired to 5V lists both, `(nano.5V, nano.D10)`. The current is the model's. Its output stage is linear, 25 ohm from its rail, so a short reads 200 mA, about twice what a real pin gives. Either is past the rating, and a real pin can be damaged there. The run does not warn at 20 mA. That is where the datasheet specifies a pin's output voltage, and above it the pin's high or low level sags, as it does here. Keep a pin at 20 mA or less when a level matters, and drive a larger load through a transistor.

## Scenes

A scene is JSONC: JSON plus `//` comments and trailing commas, nothing more. Its keys:

- `format`: always `1`.
- `parts`: each instance by name. `type` is a catalog id (the list below) or a `./` path to a part folder of your own. The other keys are the part's params. `"not_simulated": "<why>"` keeps a part in the scene for drawing but out of the simulation. Nothing may be wired to it. A wire to one of its pins is a `not_simulated` error that gives `<why>`.
- `wires`: pairs of pins, `["nano.D2", "btn1.a"]`. orbe joins wired pins into nets. Two wires to one pin are fine. A third item, `{ "kind": "jumper" }`, `{ "kind": "trace" }` or `{ "kind": "contact" }`, only says how it is drawn. `jumper` is the default.
- `nets` (optional): names a net by one of its pins, `"GND": "nano.GND"`. Other nets take the name of their smallest pin, with `.` written `__`, as `nano__GND`.
- `layout` (optional): `"btn1": { "at": [x, y, z], "rotate": [rx, ry, rz] }` in mm and degrees, for drawing only, or `"r1": { "on": { "a": "bb.d25" } }` to plug a part into a breadboard, which also wires it (see "Breadboards"). The 3D page puts a part with no `layout` in a row to the right of the placed ones, and draws its wires to it, so a scene with no `layout` at all still shows every part and wire.
- `sim` (optional): `{ "max_step": "1ms", "seed": 1 }`. `max_step` is the engine's largest step, 10 ms by default.
- `world` (optional): the room the circuit sits in, the same for every part. Each quantity holds a number, or an `initial` and timed `steps`:

  | Quantity | Unit | Range | Default |
  | --- | --- | --- | --- |
  | `temperature` | degC | -40 to 125 | 25 |
  | `humidity` | %RH | 0 to 100 | 50 |
  | `light` | lx | 0 to 100000 | 300 |

  `"world": { "light": 300, "temperature": { "initial": 25, "steps": [{ "at": "2s", "to": 30 }] } }`. A step's `at` is after 0 and within 2 hours, at least 2 ns after the step before it, and changes the value. A quantity the scene leaves out holds its default when a part reads it. `orbe set` changes one during a session.

A part name starts with a letter, takes letters, digits and `_`, and may not contain `__` or end in `_`. `i`, `v`, `p`, `pin`, `logic` and `net` are reserved. With two boards in a scene, wire their GND pins together, so both grounds are one net.

Param values:

- A number with an optional SI prefix and unit, as a string or a number, such as `"10k"`, `"4.7 kohm"`, `"220"`, `220`, `"100n"`, `"1ms"`. Prefixes: p, n, u, m, k, M.
- A choice by its name, as `"color": "red"`.
- A file by its path, relative to the file that sets it, as `"firmware": "build/button.ino.hex"`.

## Breadboards

`breadboard-400` (30 columns) and `breadboard-830` (63 columns) are parts, and every hole is one of their pins. A hole in the middle is named by its row and column, as `bb.c12`: rows `a` to `e` on one side of the centre channel, `f` to `j` on the other. A strip joins the five holes of a column on one side, so `bb.a12` to `bb.e12` are one net. Each side has two rails, and each rail joins its whole length. Beside row a, the outer rail `W` is + and the inner rail `X` is -. Beside row j, the inner rail `Y` is + and the outer rail `Z` is -. A rail hole takes the number of the column it sits beside, as `bb.X20`. Rail holes come in groups of five with a gap at every sixth column, so some numbers have no hole, such as `W6` on the 400. `orbe part` prints the holes a board has, then each strip, `;` between strips. A rail lists its groups:

```console
$ orbe part breadboard-400
...
holes  8 mm deep, each a pin with one lead; a strip joins its holes
  rows a to j at columns 1 to 30; rows W to Z at columns 1 to 5, 7 to 11, 13 to 17, 19 to 23, 25 to 29
...
  strips of 25  W1 to W5, W7 to W11, W13 to W17, W19 to W23, W25 to W29; X1 to X5, X7 to X11, X13 to X17, X19 to X23, X25 to X29; Y1 to Y5, Y7 to Y11, Y13 to Y17, Y19 to Y23, Y25 to Y29; Z1 to Z5, Z7 to Z11, Z13 to Z17, Z19 to Z23, Z25 to Z29
...
```

Give the board an `at`. Then plug each part in with `on`, one of its pins and the hole that pin's leg goes into, and turn it about Z with `rotate` in quarter turns. Its other legs land where its shape puts them, each in its own hole, and each joins that hole's strip. A resistor's legs are 4 columns apart and an LED's 1, along the row at no turn. The part sits on the board's top. A leg longer than a hole is deep, 8 mm, stands the part up on its legs: an LED's 27.4 mm legs hold its body 19.4 mm above the board.

A pin on two or more legs of one part goes in by its lowest lead. `orbe part` lists each pin's leads: a switch's `a` is on leads 1 and 2, so `"a": "bb.f22"` puts lead 1 in `f22`, and an `atmega328p-dip28`'s `GND` is on 8 and 22, so lead 8 goes in. Naming the hole another of its legs would land in moves the whole part: with `"a": "bb.e22"`, lead 1 goes in `e22`, lead 2 lands in `b22`, and the switch no longer crosses the channel. A label on legs of two parts, as the Nano's `GND` on two header pins, names no one hole, and `on` refuses it. Name a pin on one leg instead, as `D13`.

Wire the rest with jumpers between holes, and give a jumper `{ "color": "#d12b2b" }` to draw it red. The worked example on a breadboard, where the Nano straddles the channel over columns 1 to 15:

```jsonc breadboard.jsonc
{
  "format": 1,
  "parts": {
    "bb": { "type": "breadboard-400" },
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "btn1": { "type": "tactile-switch-6mm" },
    "r2": { "type": "resistor-axial", "resistance": "10k" },
    "r1": { "type": "resistor-axial", "resistance": "220" },
    "led1": { "type": "led-5mm", "color": "red" },
  },
  "layout": {
    "bb": { "at": [0, 0, 0] },
    "nano": { "on": { "D13": "bb.d1" } }, // D2 lands in j11, 5V in a12, GND in a14
    "btn1": { "on": { "a": "bb.f22" }, "rotate": [0, 0, 90] }, // a's lead 1 in f22, its lead 2 in e22; b in f20 and e20
    "r2": { "on": { "a": "bb.b22" }, "rotate": [0, 0, -90] }, // b lands in X22, the - rail
    "r1": { "on": { "a": "bb.d25" } }, // b lands in d29
    "led1": { "on": { "anode": "bb.c29" } }, // cathode lands in c30
  },
  "wires": [
    ["bb.a12", "bb.W11"], // 5V to the + rail
    ["bb.a14", "bb.X14"], // GND to the - rail
    ["bb.a20", "bb.W20"], // btn1.b to 5V
    ["bb.j11", "bb.j22"], // D2 to btn1.a
    ["bb.c1", "bb.c25"], // D13 to r1.a
    ["bb.a30", "bb.X29"], // led1.cathode to GND
  ],
}
```

It is the same circuit as `scene.jsonc`, so it runs the same. Read a hole as a pin, as `orbe get @bb.a12`. `snapshot` lists no holes, and shows a board's pins that reach another part, as with wires. A leg that lands in the channel, off the board, between holes or in a hole another leg holds is an error that names the leg and the hole. Its hint is a placement orbe tried that seats every leg with no strip joining two of the part's pins: a quarter turn from the same hole, else a hole within 10.16 mm and a turn, else it says no quarter turn seats the part there.

```console
$ orbe add breadboard-400
...
$ orbe check breadboard.jsonc
breadboard.jsonc: ok, 18 engine parts, 31 nets, ground nano__GND
```

`orbe lint` warns `scene.shorted` when two pins of one part sit in one strip, such as a resistor turned along a column, and its hint is found the same way. A wire may still end on a pin off the board, such as an Uno's header pin.

## Parts

`orbe add <id>…` copies parts from the catalog built into this `orbe` into `parts/<id>/` and records them in `parts.lock`. It also copies the parts a part is built from. Commit `parts/` and `parts.lock` with the scene. Don't edit a copied part. A copy that differs, even in spacing, is a `part_modified` error. A board built from other parts, such as `nano-board`, is the exception. An edit to it checks, with a note. A copy that matches its `parts.lock` line and is an older version than the part this orbe carries is a `part_outdated` error. The message names both versions. Its hint is one `orbe add` command that names every such id. Run it as printed, then check again. A copy that matches its lock line and is a newer version than this orbe carries is a `part_newer` error. The message names the version the project holds and the version this orbe carries. The hint says this project needs a newer orbe, and that id is not in an `orbe add` command. `--overwrite` leaves the copy where it is, including when the command writes a board that is built from it, and says this project needs a newer orbe for it. When `orbe add` refuses, it says the same for each copy it left, and the `orbe add` in its hint names every id that command would write. That command always names an id. When nothing can be written, the hint names no `orbe add` and says this project needs a newer orbe. The same version with different bytes is still `part_modified` for a sealed part: a registry refuses that change and asks for a new version. A board stays the exception above. To change a part, copy it to a folder of your own and name it by its `./` path.

`orbe add <block>` installs a Block, a small circuit that works. It copies the parts the scene names into `parts/` and records them in `parts.lock`, then writes the Block's `scene.jsonc`, `firmware/` and `tests/`. The Blocks are `nano-button-led`, `uno-button-led`, `nano-led-fade` and `nano-light-comparator`. A folder that already holds another `scene.jsonc` is `scene_exists`, since orbe does not merge scenes. A file already there with other bytes is `file_exists`. Either refusal writes nothing. A file already there with the Block's bytes is kept, so running the same install again finishes one that stopped part way. A new folder is only for a file whose bytes differ.

`orbe part <id>` prints one catalog part: its title, maker and number, license, folder hash, pins, params, ratings, what is not simulated, datasheets, where to buy it, files, the parts it is built from and the parts that use it. An LED's print names `brightness`, the average `i` over 20 ms, and the `lit`, `dim` and `off` ranges. A part with no summary or tags leaves those out. A rating set to null prints as unrated. A part with no `buy` prints `none`. A name that is not an id, and is a whole word of exactly one id, prints that part. `orbe part 1n4007` prints `diode-1n4007`. When several ids share that word, the command is `unknown_part` and the hint lists those ids. `orbe search <words>` lists parts whose id, title, maker, number, summary or tags hold every word. An exact id comes first. Then a part whose every word is a whole word of its id, its title or its tags. Then the rest. Ids break ties. `orbe part <id> --json` and `orbe search <words> --json` print the same card as JSON. Any other unknown id is `unknown_part`, and the hint names close ids.

`buy` is optional on a part. Leave it out when there is no link. An empty list is an error. Each entry has `seller`, `url` and `checked`. `checked` is the day the link was checked, as `YYYY-MM-DD`. `seller` is a name in `packages/scene/buy-sellers.jsonc`, and the URL's host is one of that seller's hosts. A new seller is a one-line addition to that file, reviewed for affiliate and tracking links. `url` is a lowercase `https://` URL with no userinfo, no whitespace, no query and no fragment. Userinfo is an `@` before the host. An entry has no price. A repeated entry is an error.

<!-- catalog parts -->

## Refs

A ref names something in the open scene. It starts with `@`, which a shell leaves alone.

- `@led1`: a part. `get` prints its state, `lit`, `dim` or `off`, its current `i`, voltage `v` and power `p`, and `brightness`, the average current over the span the reply names.
- `@nano.D13`, `@led1.anode`: a pin. `get` prints its logic level, `high` or `low`, when a chip pin is on its net, and its voltage.
- `@led1.i`: a quantity of a part: `i`, `v`, `p`, and for some parts `ib`, `ic`, `id`, `iin`, `iout`. An unknown one gets an error that lists what the part reports.
- `@nano.mcu.PB5`, `@nano.led_l`: a part inside a part, by its name in that part's file. On the Nano and the Uno the chip is `mcu`, and `led_l` is the L LED, which D13 drives.

`press @btn1` and `release @btn1` close and open a switch.

`orbe set <quantity> <value>` changes the scene's `world`, as `orbe set light 50` or `orbe set temperature 30`. The value is a plain number in the quantity's unit, inside its range. Read its effect on the circuit, such as a pin's voltage. Only a quantity the scene's `world` names, or a part reads, can be set.

## Conditions

`expect` and `step --until` take the same conditions:

- `@led1 lit`, `@led1 dim`, `@led1 off`: an LED's light state. See "LED brightness".
- `@led1.brightness > 2mA`: that average forward current, in A.
- `@nano.D13 high`, `@nano.D13 low`: a pin's logic level.
- `@led1.i > 5mA`, `@nano.D2 < 1V`: a quantity or a pin's voltage, with `<`, `<=`, `>` or `>=`, and a value in A, V or W with an optional prefix (p, n, u, m, k, M). A value as `get` prints it, written without the space, such as `2.27e-5V`, parses too.
- `serial contains "text"`: all the serial text the chip has sent since `open`, line endings included. Text printed earlier still matches. `snapshot` shows the last 10 lines of it, joined with `\n`.
- `no diagnostics`: no runtime warning has come up, such as a current over a part's or a chip pin's rating. A chip warning can show up to one engine step (10 ms) late, so step that far past its cause first.

In a shell, quote `<` and `>`, or the shell takes them as redirects, as in `orbe expect @led1.i '>' 5mA`. A test file needs no quotes for them.

## LED brightness

`brightness` is an LED's average forward current over the last 20 ms, or over the time since t = 0 when the session is younger than that. The reply names the span, as `over 20ms` or `over 14ms`. The window is 20 ms rather than the length of the `step`. A 1 ms step aliases a fade near 490 Hz, a step of seconds would smear a fade that takes seconds, and `get` has no step. `orbe part led-5mm` prints those ranges.

- `lit` while that average is above 2 mA. 2 mA is the lowest current of the forward fit, so a threshold at 1 mA would sit outside the fit.
- `dim` while that average is above 100 uA and at or below 2 mA, and `off` at or below 100 uA. 100 uA is a hundredth of the 10 mA at which the datasheet rates luminous intensity, and ten times the 10 uA reverse-current maximum. The datasheet draws luminous intensity against current as a straight line from 0 mA, so a current below the fit still glows in proportion.

Right after `step --until @led1 lit` the pin is already high and `i` is about 12 mA, while `brightness` is only about 2.4 mA, because most of the 20 ms was still dark. `expect @led1.brightness > 5mA` fails there. `step 20ms` fills the window, and `brightness` then matches `i`.

`@led1.i` shows the current 1 ps after a pin edge. The light state follows the average, so it does not change on that same edge. `step` lists a new state once it has held for 2 ms. `step --until` on that LED stops on the slice where the average crosses. `get` and `expect` read the average as it stands, with no wait.

`analogWrite` on D9 and D10 runs at about 490 Hz, a period near 2 ms. One sample of the pin aliases it. 20 ms covers about ten cycles, so `brightness` reads the duty. The fade sketch adds 5 to the duty every 30 ms and turns around at 0 and 255, so samples a few hundred milliseconds apart show the rise and the fall. Near 600 ms the LED is `lit` at about 4.7 mA. Near 1.54 s it is about 12.4 mA. Near 3.01 s it is `dim` at about 0.65 mA. `expect @led1.brightness > 3mA` and `expect @led1.brightness < 7mA` check a point on that rise. A brightness within about 0.6 mA of a threshold can read either neighbouring state, because one cycle is a small part of the window and the window can catch its edge on either side.

An LED behind 10 kohm from 5 V carries about 0.35 mA and reads `dim`. When `step --until` times out and the average covers a full 20 ms, the hint names that measurement, the state, and the range the wanted state needs: `led1 carries 0.35 mA averaged over the last 20ms and glows dimly. lit needs the average above 2 mA. Check the circuit with snapshot.` A window shorter than 20 ms says to step longer with `--timeout`, or to check the circuit with `snapshot`. The same sentence names `lit`, `dim` and `off`. At t = 0 the average is the instantaneous current, and the first full window, at 20 ms, is that current averaged over 20 ms.

## Time

- The session is paused between commands. Time moves only inside `step`, or while `play` runs (see "Watching in 3D").
- `step <duration>` advances by that much and lists what changed, such as pin edges with their exact times, LED changes and diagnostics. It lists at most 20, then `and N more`, so serial edges on D1 can hide the rest. It then lists every LED's `brightness`, and those lines are not part of the 20. `get` and `snapshot` show the state after it. Durations take ps, ns, us, ms or s, such as `100ms`, `1.5ms`, `20us`.
- A read at time t, by `get`, `expect` or `snapshot`, shows the circuit just before anything that happens at t. So `press` at t shows from t + 1 ps on, and its reply says "reads show it after t". Step after an input before you read its effect.
- An input lands at the session's time or later, and its reply gives the time. A `set` also lands after the last change the scene's `steps` schedule, so it never undoes one.
- `step --until <condition> [--timeout <duration>]` first reads the condition now. If it holds, it takes no time. Otherwise it steps until it holds, up to the timeout, 10s by default. A pin condition stops 1 ps after the exact edge. `@led1.i` shows that current there, and the light state follows the average over 20 ms. Other conditions are read every 1 ms, so the reply says `by 104ms`. A timeout is the error `until_timeout`.
- Serial can reach orbe up to one engine step (10 ms) after the chip sends it. Step a little past a print before you expect it.
- A session stops at 2 hours of simulated time. Reads still work there, and `press`, `release`, `set` and `step` fail with `ended`. A `step` that would pass the stop fails with `past_stop`. Run `orbe open` to start again.

A switch with `bounce` set bounces up to `max_bounces` times within that window after each `press` and `release`, and an input on it lands after the window of the one before. With this one-line change to the worked example's `scene.jsonc`,

```diff scene.jsonc
-    "btn1": { "type": "tactile-switch-6mm" },
+    "btn1": { "type": "tactile-switch-6mm", "bounce": "2ms" },
```

a `release` and a `press` with no `step` between land 2 ms apart. `step --until @led1 lit` stops once the average crosses 2 mA, after the first contact has opened again inside the bounce window:

```console
$ orbe open
...
$ orbe step 100ms
t=100ms (from 0s)
...
$ orbe press @btn1
pressed btn1 at t=100ms; reads show it after t=100ms
$ orbe step --until @led1 lit
t=104ms (from 100ms), @led1 lit by 104ms
  100ms nano.D2 high
  100.0040625ms nano.D13 high
  101.028757825ms nano.D2 low
...
$ orbe release @btn1
released btn1 at t=104ms; reads show it after t=104ms
$ orbe press @btn1
pressed btn1 at t=106.000002ms; reads show it after t=106.000002ms
$ orbe step 6ms
t=110ms (from 104ms)
  104ms nano.D2 low
...
$ orbe expect @led1 lit
passed @led1 lit (led1 lit, i=0.012246558123229083 A, brightness=0.004410406555696104 A over 20ms)
$ orbe close
closed at t=110ms
```

To read a bouncing switch settled, step past the time in the last input's reply plus the window. Here that is 106.000002ms + 2ms, so `step 6ms` from 104ms.

## Test files

A test file is `tests/<name>.test` in the project folder. Each line is a live command without `orbe`. `--json`, `--session`, `--help` or `-h` on a line is a usage error, except as the text after `serial contains`. Live, `--help` anywhere prints help, so `orbe expect serial contains --help` prints help, while the same test line looks for that text. `#` starts a comment. `"..."` holds spaces, and inside it only `\"` and `\\` escape, so a test cannot name a line ending. To tell lines apart, print text that only one line has, such as `led=on`. Paths start from the project folder, and `open` alone opens `scene.jsonc`.

`orbe test` runs every `tests/*.test` of the project in the current folder, each in a fresh session, and stops a file at its first failed line. `orbe test <file or folder>…` runs those. `-v` prints every command with its reply. A failure prints the line, the expected and the actual value. With the first `expect` in `tests/button.test` changed to `expect @led1 lit`:

```
FAIL tests/button.test:4: expect @led1 lit
  error[expect_failed]: expected @led1 lit
    expected: @led1 lit
    actual:   led1 off, i=1.341311570535588e-13 A, brightness=1.3413115705356304e-13 A over 20ms
0 passed, 1 failed
```

A test needs no background process, so it runs the same in CI. Each file's session lives inside `orbe test` and ends with it. A session that `orbe open` started keeps running, untouched. `orbe test` takes no `--session` and ignores `ORBE_SESSION`.

## Warnings

`orbe lint` prints `warning[<code>]` with a `hint:` line for what checks but is likely wrong or unclear. Warnings never fail it. When the scene does not check, it prints and exits as `orbe check` does, with no warnings. It writes no file. `orbe open` shows none of these warnings except `net.floating`.

| Code | What it means, and what to do |
| --- | --- |
| `scene.led_no_resistor` | An LED joins two driven nets, such as a pin and GND, with no resistor in series, so it can draw far over its rating. The path runs through diodes, switches, and a transistor whose emitter is on a driven net, as in a low-side NPN switch. A chip pin on a net that a resistor also feeds from a driven net counts as reading that net, not driving it. So A0 wired to read an LED's node does not hide its series resistor, and lint misses an LED wired straight to an output pin that also has a pull-up. Put a resistor in series, such as 220 ohm from a 5 V pin. |
| `scene.no_model` | A part has no engine model, no `not_simulated` reason and no `"joins_only": true`, so the simulation runs nothing for it. Use a part with a model. A part of your own that only joins the wires that reach it, such as a jumper, takes `"joins_only": true` in its part file. A part nothing is wired to can take `"not_simulated": "<why>"` in the scene instead. |
| `scene.unconnected` | Nothing is wired to a part, or a part with at most 3 nets, such as a resistor, LED, transistor or pot, has a net with no wire. Wire it, or leave it open when the circuit means it, as a pot used as a variable resistor. A board runs its chip from its own supply, so a board alone, such as one blinking its L LED, gets no warning. |
| `net.floating` | A net has no DC path to ground. Fix it when a part reads that net. |
| `scene.ground_unnamed` | The ground net is named after a pin. Name it in `nets`, as `"GND": "nano.GND"`. |
| `scene.unplaced` | Parts have no `layout` position, so the 3D page places them in a row for drawing only. One warning names them all. A part plugged in with `on` has a position. Give a part a position when its place matters. |
| `scene.shorted` | Two pins of one part sit in one breadboard strip, so the strip joins them, such as a resistor turned along a column. Turn the part a quarter turn, as the hint shows, or plug it in elsewhere. |
| `scene.rotation` | A `rotate` angle is outside -180 to 180 degrees. Write the same turn as the hint shows. |

## Output and exit codes

Every command prints text. With `--json` it prints one JSON object instead, `{"ok": true, "data": {...}}` or `{"ok": false, "error": {"code", "message", "hint"}}`, where `hint` can be missing or empty. A failed `orbe check scene.jsonc --json` is `scene_invalid`, with each finding, its code and hint in `data.diagnostics`. `orbe lint scene.jsonc --json` on a scene that checks is `ok`, with each warning in `data.diagnostics` in the same shape. Live commands add `session`, `t_ps` (picoseconds) and `state`: `paused`, `playing`, `stopped`, `faulted` or `closed`.

Exit codes: 0 ok, 1 a failed check, `expect` or command, 2 a usage error, 3 a file or system error. `orbe test` exits with the worst file's code.

## Sessions

- `orbe open` starts a background process that holds the session, and every later command talks to it. The process ends on `orbe close`, or after 1 hour with no command while it is not playing and no page watches it.
- A session has a name. By default it comes from the project folder, the nearest one up with `scene.jsonc` or `parts.lock`, so each project gets its own, and its `tests/` folder shares it. Outside a project it comes from the git repository's root folder, else the current folder. `--session <name>` after the command, as `orbe open --session bench`, or `ORBE_SESSION=<name>`, picks another, so two sessions can run side by side. Live paths start from the shell's current folder, so run live commands from the project folder.
- `orbe close --all` closes every session of this `orbe` and cleans up after crashed ones.
- A command sent while a `step` runs waits for it. Ctrl-C during a long `step` stops it, and the session stays paused at the time it reached.

## Watching in 3D

A person can watch a live session in 3D and press its buttons. `orbe open --headed` opens the scene, then a page in the default browser. `orbe show` opens the page on a session that already runs. Each prints the page's address, which only this machine can reach, and `--no-browser` prints it without opening a browser. The address holds the session's key, so give it only to the user who asked to watch. On another machine it works only through a tunnel to this one, such as `ssh -L <port>:127.0.0.1:<port>`, opened at `http://127.0.0.1:<port>/` with the same `#t=` key.

- Several pages can watch one session. Closing a page leaves the session running, and `orbe close` ends every page.
- A click on a button in the page is `press`, and letting go is `release`. They land at the session's time, as yours do, so read the session with `get` or `snapshot` as always. A session opens paused, and while it is paused a click changes no pin, because the firmware runs only as time moves. Before you hand a person the page, run `orbe play`, or tell them to press Play.
- `orbe play` runs time at real speed and returns at once, and `state` is `playing` meanwhile. Time runs until `orbe pause`, a `step` that runs, the 2-hour stop or a fault. A refused `step`, such as `past_stop` or an `unknown_ref` in `--until`, leaves it running. The page's Play button runs the same `play`.
- `play` reports nothing as time runs. The next `step` lists the changes since the `step` before it, play included, such as the pin edges, LED changes and diagnostics a press from the page caused, so its reply reads like `t=12.001s (from 2s)`. Like any step it lists the first 20, then `and N more`, so after a busy play the step's own changes can fall into that count. Run a short `step 1ms` first to list play's, then the step you meant. When play ran into the 2-hour stop or a fault, the `step` that is refused lists them. `pause` lists nothing, and `get` and `snapshot` read the state at any time.
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
| `scene_exists`, `file_exists` | `orbe add <block>` found another scene, or a file whose bytes differ. Matching files are kept, and running it again finishes an install that stopped part way. Use a new folder only when the bytes differ, as the hint shows. |
| `symlink` | `orbe add <block>` found a link where it would write. It does not follow the link. Remove the link, then run the command in the hint again. |
| `firmware_missing` | Build the firmware, or fix the path. It starts from the scene's folder. |
| `not_simulated` | A wire reaches a pin the engine does not simulate, or a part marked `not_simulated`. Wire another pin, or remove the wire. |
| `scene.unknown_pin`, `scene.param`, `scene.syntax` | A typo or a wrong value at that line and column. The hint names what is valid. |
| `scene.world` | A `world` quantity that does not exist, a value outside its range, or a step at 0, too close to the one before it, or that does not change the value. The hint names what is valid. |
| `part_outdated` | The copy matches its `parts.lock` line and is an older version than this orbe carries. The message names both versions. Run the one `orbe add` command in the hint. It names every older id, and no newer one. |
| `part_newer` | The copy matches its `parts.lock` line and is a newer version than this orbe carries. The message names both versions. This project needs a newer orbe. The hint names no `orbe add` command. `--overwrite` leaves the copy, and says so. |
| `part_modified` | A copied part was edited. Run `orbe add <id> --overwrite` to restore it, or copy it to a folder of your own. |
| `part_format` | The part file's format is not one this orbe reads. An older catalog copy is written for an older orbe: run `orbe add <id> --overwrite`. A part opened by a `./` path is the user's file: update that file to the format this orbe reads, including its model, or point the scene at the catalog id if the file copies one. A newer part means this project needs a newer orbe. |
| `no_session` | No session runs under this name. Run the `orbe open` in the hint. |
| `session_crashed` | The session's process died. Run `orbe open` again. |
| `unknown_ref`, `no_reading` | The ref names nothing, or nothing to read. The hint lists what exists. |
| `not_settable` | `press` and `release` work only on a switch, by its part name. `set` works only on a quantity the scene's `world` names or a part reads. |
| `no_change` | The input asks for the state already there, such as `set light 50` when the light is already 50. |
| `expect_failed`, `until_timeout` | The condition did not hold. Compare `actual`. When an LED timeout covers a full 20 ms, the hint names the measured average, the state, and the range the wanted state needs, and says to check the circuit with `snapshot`. A shorter window says to step longer. Otherwise fix the circuit or the firmware, or step longer. |
| `ended`, `past_stop` | The session reached its 2-hour stop. Run `orbe open`. |
| `cancelled` | Ctrl-C stopped a `step`. The session stays at the time it reached. Step again. |
| `faulted` | The engine stopped the session. The message names why. Fix it and run `orbe open`. |

## What is not simulated

- The Nano's 3V3, VIN, A6 and A7 pins. A wire to one is a `not_simulated` error.
- The Uno's VIN pin and barrel jack. A wire to VIN is a `not_simulated` error. The Uno's 3V3 works.
- The Uno's TX and RX LEDs and the gate of its USB switch. D13 feeds its L LED through 1k, so D13 as an input with its pull-up reads low.
- USB, and serial into the chip. The chip's serial out (TX, D1) is read as text.
- The chip's clamp diodes and its own supply current. A pin pulled past VCC or GND draws no clamp current, and `rating.voltage` names it. The current through the chip's VCC and GND is its pins' alone.
- A switch's force and travel, and part tolerances. An LED's `brightness` is its average forward current over the last 20 ms. It reads `lit` above 2 mA, `dim` above 100 uA and at or below 2 mA, and `off` at or below 100 uA. Light, temperature and humidity reach only a part whose file reads them from the scene's `world`.
- Turning a potentiometer during a session. Its `position` param sets the wiper for the whole run, so run `orbe open` again after you change it.
- Any part the catalog above does not list.
- orbe never builds firmware. Build it with your toolchain and name the output file in the scene.
