# orbe: ratings

Load this when a warning names a rating, or an LED is lit, dim or off. `orbe part <id>` prints that part's ratings. A rating set to null prints as unrated.

## An LED with no resistor

With D13 wired straight to `led1.anode` and no series resistor, the LED draws about 94 mA. `step` lists `warning[rating.current]` for `led1`, and `#led1.p` and `#nano__mcu.pin.PB5 (nano.D13)`, the chip pin behind D13, as over now. `step 20ms` after that lists their sustained warnings. `expect no diagnostics` fails. The LED holds D13 near 2.3 V, so D13 reads `low` while the LED is lit. `orbe lint` warns first, with `warning[scene.led_no_resistor]` on `led1`. Give each LED a series resistor, such as 220 ohm from a 5 V pin.

## The chip and the USB supply

Each current and power rating has its datasheet's basis, which `orbe part` prints. A peak warns on any sample over it. A sustained rating warns when an RMS or mean filter with its time constant, starting at zero, passes it. An LED's or a diode's current also has its sheet's pulse or surge peak. A pulse within that peak and a 490 Hz PWM stay quiet. The USB supply's 500 mA is a peak.

The board's ATmega328P uses the datasheet's absolute maximum ratings, a pin's current and the supply's current as a 10.2 ms RMS. A run warns `rating.current` on a pin that sources or sinks over 40 mA, on `VCC` or `GND` when the pins together pass over 200 mA, and `rating.voltage` on a pin below -0.5 V or above VCC + 0.5 V. The message names the chip pin and the board pins, as in `#nano__mcu.pin.PB1 (nano.D9) carries 43.4 mA RMS, over its 40 mA rating`. A lasting warning ends with how long, as in `for 32.6ms`.

A load switched on over a sustained rating with no peak is listed at once as over now, as in `#r1.p is 0.989 W now, over its 0.25 W rating; it warns if held about 2ms`. It is over now while the last 2 ms averages over, so a PWM whose average is under stays quiet. A diode or zener is never over now on power: the current peak warns at once past the surge, and the power filter warns when heat crosses. An LED still is. `expect no diagnostics` fails then. In `--json`, an over-now warning has `data.now` true. The pin's output stage is linear, 25 ohm from its rail. Keep a pin at 20 mA or less when a level matters, and drive a larger load through a transistor.

The board's USB supply is rated 500 mA. A load over that on 5V (e.g. 2.5 Ω, ~1.8 A) warns `rating.current` on `#nano__usb__host.i (nano.usb)`, and ~18 ms later on the 0.5 A Schottky `#nano__diode.i (nano.diode)`. `neopixel-stick-8` at full white draws 296 mA (quiet); `neopixel-ring-16` draws 586 mA and warns unless powered from an external 5 V supply. A wire from 5V to GND is a short (`net.short`).

## LED brightness

`brightness` is an LED's average forward current over the last 20 ms, or over the time since t = 0 when the session is younger. The reply names the span, as `over 20ms`. A red 5 mm LED reads `lit` above 2 mA, `dim` above 100 uA and at or below 2 mA, and `off` at or below 100 uA. Another colour can use other currents. `orbe part` prints them. `step` lists a new state once it has held for 2 ms. `step --until` on an LED stops on the slice where the average crosses.

Right after `step --until @led1 lit` the pin is already high and `i` is about 12 mA, while `brightness` is only about 2.4 mA, because most of the 20 ms was still dark. `step 20ms` fills the window, and `brightness` then matches `i`. Under `analogWrite` the window covers about ten PWM cycles, so `brightness` reads the duty. A brightness within about 0.6 mA of a threshold can read either neighbouring state. A red 5 mm LED behind 10 kohm from 5 V carries about 0.35 mA and reads `dim`. When `step --until` on an LED times out, the hint names the average, the state and the range the wanted state needs.

`@led1.color` is the colour an addressable LED last latched, as `rgb(r,g,b)` from 0 to 255, and when. `get @led1` prints it as `color=`. For addressable RGB LEDs, load `orbe skills get parts`.

## Warnings

`orbe lint` prints `warning[<code>]` and a `hint:`. Warnings never fail it. `orbe open` shows only `net.floating`.

| Code | What it means, and what to do |
| --- | --- |
| `scene.led_no_resistor` | An LED joins two driven nets with no series resistor. Put one in series (e.g. 220 ohm from 5 V). A relay contact counts as a switch; a pin with a pull-up is missed. |
| `scene.no_model` | No engine model, `not_simulated` reason or `"joins_only": true`. Use a part with a model. |
| `scene.unconnected` | Nothing is wired to a part, or a part with at most 3 nets has an open net. Wire it or leave it open if meant. |
| `net.floating` | No DC path to ground. Fix it when a part reads that net. `orbe nets` marks `(floating)`. |
| `scene.unplaced` | Missing `layout` (drawn in a row), or `at` puts a leg in a hole without joining it. Seat with `on`, or a contact wire when a hole is held; non-zero `rotate` stays. |
| `scene.shorted` | Two pins of one part share a breadboard strip. Turn the part a quarter turn. |
| `scene.covered_hole` | A wire end or seated lead sits in a hole another part's body covers. Move the wire to a clear hole in that strip, or move a part. |
| `scene.overlap` | Two placed parts overlap in space. Move one in `layout` so their bodies clear. |

`warning[rating.current]`, `rating.power` or `rating.voltage` means a part passed its sheet rating; `rating.supply_low` means a chip supply sat above 0.1 V and below its sheet minimum (clean off-to-on or on-to-off transitions stay silent; sags, incomplete turn-ons, or rails starting or ending in the band warn); `regulator.current_limit` means a regulator hit its limit and sagged; `rating.temperature`, the scene is past a sensor's sheet (DHT22 80 °C, BMP280 85 °C). Fix it. `chip.floating_input` names an undriven input (`arduino`).

Reverse voltage warns past 1 V on electrolytics and 2.4 V on tantalums. Regulator inputs warn below -0.3 V or past their max (35 V on LM7805); op-amp supplies warn past max. Reversed 9 V on Nano VIN warns `#nano__regulator.pin.in (nano.VIN)`; a series 1N4007 holds it near 0 V. USB-only VIN stays near 0.
