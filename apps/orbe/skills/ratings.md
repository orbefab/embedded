# orbe: ratings

Load this when a warning names a rating, or an LED is lit, dim or off. `orbe part <id>` prints that part's ratings. A rating set to null prints as unrated.

## An LED with no resistor

With D13 wired straight to `led1.anode` and no series resistor, the LED draws about 94 mA. `step` lists `warning[rating.current]` for `led1`, and `#led1.p` and `#nano__mcu.pin.PB5 (nano.D13)`, the chip pin behind D13, as over now. `step 20ms` after that lists their windowed warnings. `expect no diagnostics` fails. The LED holds D13 near 2.3 V, so D13 reads `low` while the LED is lit. `orbe lint` warns first, with `warning[scene.led_no_resistor]` on `led1`. Give each LED a series resistor, such as 220 ohm from a 5 V pin.

## The chip and the USB supply

The board's ATmega328P uses the datasheet's absolute maximum ratings. A pin's current and the supply's current are the RMS over the last 10.2 ms, five periods of `analogWrite`'s 490 Hz PWM, so a PWM load counts by its RMS, not its peak. Power is the average over the same 10.2 ms, so a short switching spike stays under a power rating and a held overload warns. Each window starts at zero when the session opens. A run warns `rating.current` on a pin that sources or sinks over 40 mA, on `VCC` or `GND` when the pins together pass over 200 mA, and `rating.voltage` on a pin below -0.5 V or above VCC + 0.5 V. The message names the chip pin, then the board's pins on its net, as in `#nano__mcu.pin.PB1 (nano.D9) carries 43.4 mA RMS over the last 10.2 ms while it is 69.4 mA, over its 40 mA rating`. Once a warning has lasted, the message ends with how long, as in `for 32.6ms`. Values are rounded to 3 significant digits.

A load switched on over a windowed rating is listed at once as over now, as in `#r1.p is 0.989 W now, over its 0.25 W rating; it warns if held about 1.6ms`. It is over now while it is over and has been for most of the last 2 ms, so a PWM whose RMS is under stays quiet. `expect no diagnostics` fails while a rating is over now. The windowed warning follows when the window crosses. In `--json`, an over-now warning has `data.now` true. The pin's output stage is linear, 25 ohm from its rail. Keep a pin at 20 mA or less when a level matters, and drive a larger load through a transistor.

The board's USB supply is rated 500 mA. A load over that on 5V, such as 2.5 ohm from 5V to GND (about 1.8 A), warns `rating.current` on the supply, `#nano__usb__host.i (nano.usb)`, and on the Schottky behind 5V, `#nano__diode.i (nano.diode)`, which is rated 0.5 A too. A wire from the Nano's 5V straight to GND is a short. `orbe check` and `orbe open` refuse it with `net.short` and name the wire.

## LED brightness

`brightness` is an LED's average forward current over the last 20 ms, or over the time since t = 0 when the session is younger. The reply names the span, as `over 20ms`. It reads `lit` above 2 mA, `dim` above 100 uA and at or below 2 mA, and `off` at or below 100 uA. `step` lists a new state once it has held for 2 ms. `step --until` on an LED stops on the slice where the average crosses.

Right after `step --until @led1 lit` the pin is already high and `i` is about 12 mA, while `brightness` is only about 2.4 mA, because most of the 20 ms was still dark. `step 20ms` fills the window, and `brightness` then matches `i`. Under `analogWrite` the window covers about ten PWM cycles, so `brightness` reads the duty. A brightness within about 0.6 mA of a threshold can read either neighbouring state. An LED behind 10 kohm from 5 V carries about 0.35 mA and reads `dim`. When `step --until` on an LED times out, the hint names the average, the state and the range the wanted state needs.

`@led1.color` is the colour an addressable LED last latched, as `rgb(r,g,b)` from 0 to 255, and when. `get @led1` prints it as `color=`. For addressable RGB LEDs, load `orbe skills get parts`.

## Warnings

`orbe lint` prints `warning[<code>]` and a `hint:`. Warnings never fail it. `orbe open` shows none of these except `net.floating`.

| Code | What it means, and what to do |
| --- | --- |
| `scene.led_no_resistor` | An LED joins two driven nets, such as a pin and GND, with no resistor in series. Put one in series, such as 220 ohm from a 5 V pin. Lint misses an LED wired straight to an output pin that also has a pull-up. |
| `scene.no_model` | A part has no engine model, no `not_simulated` reason and no `"joins_only": true`, so nothing runs for it. Use a part with a model. |
| `scene.unconnected` | Nothing is wired to a part, or a part with at most 3 nets has a net with no wire. Wire it, or leave it open when the circuit means it. |
| `net.floating` | A net has no DC path to ground. Fix it when a part reads that net. `orbe nets` marks `(floating)`. |
| `scene.unplaced` | Parts have no `layout`, so the 3D page puts them in a row. It names the leg an `at` draws in a hole that joins nothing there. Seat it with `on`, or a contact wire when `on` would claim a held hole. A non-zero `rotate` stays. |
| `scene.shorted` | Two pins of one part sit in one breadboard strip, such as a resistor turned along a column. Turn the part a quarter turn, as the hint shows. |

A `warning[rating.current]`, `warning[rating.power]` or `warning[rating.voltage]` during `step` or `open` means a part or a chip pin went past its datasheet rating. Fix the circuit. `chip.floating_input` names an input nothing drives. Load `orbe skills get arduino`.

An aluminum electrolytic warns `rating.voltage` past 1 V reverse. A tantalum warns `rating.voltage` past 25 mV reverse. A regulator input warns below -0.3 V. Reversed 9 V on Nano VIN warns `#nano__regulator.pin.in (nano.VIN)`. A series 1N4007 holds it near 0 V; 9 V forwards makes 5 V. Rail tantalums stay out, and output-to-input current stays blocked, so USB-only VIN stays near 0.
