# orbe: reference

The details behind `orbe skills get core`: breadboards in full, ratings, LED brightness, time, test files, sessions, the 3D page, and the warnings and errors an agent meets. The examples build on the core skill's worked example.

## Breadboards

A hole in the middle is named by its row and column, as `bb.c12`. A strip joins the five holes of a column on one side, so `bb.a12` to `bb.e12` are one net. Each side has two rails, and each rail joins its whole length. Beside row a, the outer rail `W` is + and the inner rail `X` is -. Beside row j, the inner rail `Y` is + and the outer rail `Z` is -. A rail hole takes the number of the column it sits beside, as `bb.X20`. Rail holes come in groups of five with a gap at every sixth column, so some numbers have no hole, such as `W6` on the 400:

```console
$ orbe part breadboard-400
...
holes  8 mm deep, each a pin with one lead; a strip joins its holes
  rows a to j at columns 1 to 30; rows W to Z at columns 1 to 5, 7 to 11, 13 to 17, 19 to 23, 25 to 29
...
```

A part seated with `on` sits on the board's top, and each of its legs joins its hole's strip. A resistor's legs are 4 columns apart and an LED's 1, along the row at no turn. A leg longer than a hole is deep stands the part up on its legs: an LED's 27.4 mm legs hold its body 19.4 mm above the board.

A pin on two or more legs of one part goes in by its lowest lead. `orbe part` lists each pin's leads: a switch's `a` is on leads 1 and 2, so `"a": "bb.f22"` puts lead 1 in `f22`. Naming the hole another of its legs would land in moves the whole part: with `"a": "bb.e22"`, lead 1 goes in `e22`, lead 2 lands in `b22`, and the switch no longer crosses the channel. A label on legs of two parts, as the Nano's `GND` on two header pins, names no one hole, and `on` refuses it. Name a pin on one leg instead, as `D13`.

The worked example on a breadboard, with the Nano across the channel over columns 1 to 15. A jumper with `{ "color": "#d12b2b" }` draws red.

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

```console
$ orbe add breadboard-400
...
$ orbe check breadboard.jsonc
breadboard.jsonc: ok, 6 scene parts, expanding to 20 engine parts, 35 nets, ground GND
```

Read a hole as a pin, as `orbe get @bb.a12`. `snapshot` lists no holes. A leg that lands in the channel, off the board, between holes or in a hole another leg holds is an error that names the leg and the hole. Its hint is a placement that seats every leg with no strip joining two of the part's pins: a quarter turn from the same hole, else a hole within 10.16 mm and a turn, else it says no quarter turn seats the part there. A hole holds one lead, so a wire end in a hole a leg or another wire end already names is an error that names both. Its hint lists every free hole in that strip, in order, as `a hole holds one lead. free holes in this strip: bb.c1, bb.e1`, or says `a hole holds one lead, and this strip has no free hole`. A wire may still end on a pin off the board, such as an Uno's header pin.

## Ratings

With D13 wired straight to `led1.anode` and no `r1`, the LED draws 94 mA. `step` lists `warning[rating.current]` and `warning[rating.power]` for `led1`, and `step 20ms` after it lists `warning[rating.current]` for `#nano__mcu.pin.PB5 (nano.D13)`, the chip pin behind D13, over its 40 mA rating. `expect no diagnostics` fails. The LED holds D13 at 2.3 V, so D13 reads `low` while the LED is lit. `orbe lint` warns of it before any run, with `warning[scene.led_no_resistor]` on `led1`. Give each LED a series resistor, such as 220 ohm from a 5 V pin.

The board's ATmega328P has the datasheet's absolute maximum ratings. A pin's and the supply's current count by their RMS over the last 10.2 ms, five periods of `analogWrite`'s 490 Hz PWM, so a PWM load counts by its RMS, not its peak. A run warns `rating.current` on a pin that sources or sinks over 40 mA, on `VCC` or `GND` when the pins together pass over 200 mA, and `rating.voltage` on a pin below -0.5 V or above VCC + 0.5 V. The message names the chip pin, then the board's pins on its net, as in `#nano__mcu.pin.PB1 (nano.D9) carries 43.4 mA RMS over the last 10.2 ms, over its 40 mA rating`. Once a warning has lasted, its message ends with how long, as in `for 32.582912ms`. The pin's output stage is linear, 25 ohm from its rail. Keep a pin at 20 mA or less when a level matters, and drive a larger load through a transistor.

The board's USB supply is rated at 500 mA. A load over that on 5V, such as 2.5 ohm from 5V to GND (1.8 A), warns `rating.current` on the supply, `#nano__usb__host.i (nano.usb)`, at 1.8 A over its 0.5 A rating, and on the Schottky behind 5V, `#nano__diode.i (nano.diode)`, which is rated 0.5 A too. A wire from the Nano's 5V straight to GND is a short: `orbe check` and `orbe open` refuse the scene with `net.short`, naming the wire.

## LED brightness

`brightness` is an LED's average forward current over the last 20 ms, or over the time since t = 0 when the session is younger. The reply names the span, as `over 20ms`. It reads `lit` above 2 mA, `dim` above 100 uA and at or below 2 mA, and `off` at or below 100 uA. `step` lists a new state once it has held for 2 ms, and `step --until` on an LED stops on the slice where the average crosses.

Right after `step --until @led1 lit` the pin is already high and `i` is about 12 mA, while `brightness` is only about 2.4 mA, because most of the 20 ms was still dark. `step 20ms` fills the window, and `brightness` then matches `i`. Under `analogWrite` the 20 ms window covers about ten PWM cycles, so `brightness` reads the duty. A brightness within about 0.6 mA of a threshold can read either neighbouring state. An LED behind 10 kohm from 5 V carries about 0.35 mA and reads `dim`. When `step --until` on an LED times out, the hint names the average, the state and the range the wanted state needs.

## Time

- `step <duration>` takes ps, ns, us, ms or s, such as `100ms`, `1.5ms`, `20us`. Serial edges on D1 can fill its 20 lines.
- An input lands at the session's time or later, and its reply gives the time. A `set` also lands after the last change the scene's `steps` schedule, so it never undoes one.
- `step --until` first reads the condition now. If it holds, it takes no time. A pin condition stops 1 ps after the exact edge. `@led1.i` shows that current there, and the light state follows the average over 20 ms. Other conditions are read every 1 ms, so the reply says `by 104ms`.
- Serial can reach orbe up to one engine step (`sim.max_step`, 10 ms by default) after the chip sends it. Step a little past a print before you expect it.
- A session stops at 2 hours of simulated time. Reads still work there, and `press`, `release`, `set`, `step` and `play` fail with `ended`. `step <duration>` past the stop fails with `past_stop`, and its hint names the time left. `step --until` stops there and fails with `ended`. An input that would land past the stop fails with `past_stop`, and its hint says to run `orbe open`.

A switch with `bounce` set bounces up to `max_bounces` times within that window after each `press` and `release`, and an input on it lands after the window of the one before. With this change to the worked example's `scene.jsonc`,

```diff scene.jsonc
-    "btn1": { "type": "tactile-switch-6mm" },
+    "btn1": { "type": "tactile-switch-6mm", "bounce": "2ms" },
```

a `release` and a `press` with no `step` between land 2 ms apart. `step --until @led1 lit` stops once the average crosses 2 mA, after the first contact has opened again inside the bounce window:

```console
$ orbe open
...
$ orbe step 100ms
...
$ orbe press @btn1
pressed btn1 at t=100ms; reads show it after t=100ms
$ orbe step --until @led1 lit
t=105ms (from 100ms), @led1 lit by 105ms
  100ms nano.D2 high
  100.0040625ms nano.D13 high
  101.028757825ms nano.D2 low
...
$ orbe release @btn1
released btn1 at t=105ms; reads show it after t=105ms
$ orbe press @btn1
pressed btn1 at t=107.000002ms; reads show it after t=107.000002ms
$ orbe step 6ms
...
$ orbe expect @led1 lit
passed @led1 lit (led1 lit, i=11.292504375073456 mA, brightness=4.627811344963918 mA over 20ms)
$ orbe close
closed at t=111ms
```

To read a bouncing switch settled, step past the time in the last input's reply plus the window. Here that is 107.000002ms + 2ms, so `step 6ms` from 105ms.

## Test files

Each line is a live command without `orbe`. `--json`, `--session`, `--help` or `-h` on a line is a usage error, except as the text after `serial contains`. Live, `--help` anywhere prints help, so `orbe expect serial contains --help` prints help, while the same test line looks for that text. `"..."` holds spaces, and inside it only `\"` and `\\` escape, so a test cannot name a line ending. To tell lines apart, print text that only one line has, such as `led=on`. A test file refuses `play` and `pause` with `usage`. `orbe test <file or folder>…` runs only those.

## Sessions

- `orbe open` starts a background process that holds the session, and every later command talks to it. The process ends on `orbe close`, or after 1 hour with no command while it is not playing and no page watches it.
- A session has a name. By default it comes from the project folder, the nearest one up with `scene.jsonc` or `parts/`, so each project gets its own, and its `tests/` folder shares it. Outside a project it comes from the git repository's root folder, else the current folder. `--session <name>`, or `ORBE_SESSION=<name>`, picks another.
- `orbe close --all` closes every session and cleans up after crashed ones. `session_crashed` names `<name>.log.crashed` when the process left output. A failed `open` after an earlier crash names `<name>.log`, the retry's output. A crash that wrote nothing removes it.
- A command other than `close` refuses a session another orbe version started, as `version_mismatch`. Run `orbe close`.
- A command sent while a `step` runs waits for it. Ctrl-C during a long `step` stops it, and the session stays paused at the time it reached.

## The 3D page

Only this machine reaches the page. From another, `ssh -L <port>:127.0.0.1:<port>` and open `http://127.0.0.1:<port>/` with the same `#t=` key. Several pages watch one session; `orbe close` ends them. A click is `press`; letting go is `release`. While paused a click changes no pin: the firmware runs only as time moves. `play` reports nothing as time runs. `pause`, or the next `step`, lists what changed while playing, including what the user did on the page. `show` failing with "this orbe was built without the viewer page" means this `orbe` has no page. Tell the user to rebuild `orbe`. The page's glow is the mean current since the last frame, so a PWM-dimmed LED glows at its average.

## Warnings

`orbe lint` prints `warning[<code>]` with a `hint:` line for what checks but is likely wrong or unclear. Warnings never fail it. `orbe open` shows none of these warnings except `net.floating`.

| Code | What it means, and what to do |
| --- | --- |
| `scene.led_no_resistor` | An LED joins two driven nets, such as a pin and GND, with no resistor in series. Put one in series, such as 220 ohm from a 5 V pin. Lint misses an LED wired straight to an output pin that also has a pull-up. |
| `scene.no_model` | A part has no engine model, no `not_simulated` reason and no `"joins_only": true`, so nothing runs for it. Use a part with a model. |
| `scene.unconnected` | Nothing is wired to a part, or a part with at most 3 nets has a net with no wire. Wire it, or leave it open when the circuit means it. |
| `net.floating` | A net has no DC path to ground. Fix it when a part reads that net. |
| `scene.unplaced` | Parts have no `layout` position, so the 3D page puts them in a row. It names the leg an `at` draws in a hole that joins nothing there. Seat it with `on`, or a contact wire when `on` would claim a held hole. A non-zero `rotate` stays. |
| `scene.shorted` | Two pins of one part sit in one breadboard strip, such as a resistor turned along a column. Turn the part a quarter turn, as the hint shows. |

## Errors

The part and `orbe add` codes are in `orbe skills get parts`.

| Code | What to do |
| --- | --- |
| `usage` | The command line is wrong. The message says how. |
| `firmware_missing` | Build the firmware, or fix the path. It starts from the scene's folder. |
| `not_simulated` | A wire reaches a pin the engine does not simulate, or a part marked `not_simulated`. Wire another pin, or remove the wire. |
| `scene.unknown_pin`, `scene.param`, `scene.syntax` | A typo or a wrong value at that line and column. The hint names what is valid. |
| `scene.world` | A `world` quantity that does not exist, a value outside its range, or a step at 0, too close to the one before it, or that does not change the value. |
| `no_session` | No session runs under this name. Run the `orbe open` in the hint. |
| `session_crashed` | The session's process died. Run `orbe open` again. It names `<name>.log.crashed` when the process left output. A failed `open` after an earlier crash names `<name>.log`, the retry's output. A crash that wrote nothing removes the older one. |
| `version_mismatch` | Another version of orbe started this session. Run `orbe close`. |
| `unknown_ref`, `no_reading` | The ref names nothing, or nothing to read. The hint lists what exists. |
| `not_settable` | `press` and `release` work only on a switch. `set` works only on a quantity the scene's `world` names or a part reads. |
| `value` | A live param set out of range. |
| `wrong_setting` | The part takes a different input, such as a trimmer's `position`. Set that param in the scene and run `orbe open` again. |
| `no_change` | `press` or `release` asks for the switch state already there. `set` of a value already scheduled succeeds, and names the time it lands when that time is still ahead. |
| `expect_failed`, `until_timeout` | The condition did not hold. Compare `actual`, then fix the circuit or the firmware, or step longer. |
| `ended` | The session reached its 2-hour stop. Reads still work. Run `orbe open` to start again. |
| `past_stop` | A `step` would end past the stop, or an input would land past it. Do what the hint says. |
| `cancelled` | Ctrl-C stopped a `step`. Step again. |
| `faulted` | The engine stopped the session. The message names why. Fix it and run `orbe open`. |

## Turning a knob

`set` turns a knob while the session runs. Its wiper feeds A0; D13 lights over half scale:

```text tests/knob.test
open
set @pot1.position 0.8
step 100ms
expect @led1 lit
```

## What is not simulated

`orbe part <id>` lists what each part leaves out, and `orbe skills get arduino` what each board's chip does.

- The Nano's 3V3 pin. A wire to it is a `not_simulated` error. VIN feeds the AMS1117-5.0, and a wire to VIN works.
- The Uno's VIN pin and barrel jack. The Uno's 3V3 works.
- USB, and serial into the chip. The chip's serial out (TX, D1) is read as text.
- The chip's clamp diodes. A pin pulled past VCC or GND draws no clamp current, and `rating.voltage` names it.
- A switch's force and travel, and part tolerances.
- Turning a potentiometer during a session. Its `position` param sets the wiper for the whole run.
- orbe never builds firmware. Build it with your toolchain and name the output file in the scene.
