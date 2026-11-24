# orbe: breadboards

Load this when seating a part, naming a hole, or fixing a breadboard error. `orbe part breadboard-400` prints every hole. `breadboard-400` has 30 columns and `breadboard-830` has 63.

A hole is a pin, as `bb.c12`. Rows `a` to `e` sit on one side of the centre channel and `f` to `j` on the other. The five holes of a column on one side are one strip, so one net. Beside row `a`, rail `W` is + and `X` is -. Beside row `j`, `Y` is + and `Z` is -. A rail hole takes the column beside it, as `bb.X20`. Rail holes come in groups of five with a gap at every sixth column, so some numbers have no hole, such as `W6` on the 400.

Give the board an `at`, `[x, y, z]` in mm, and a `rotate` in degrees. Seat each part that has leads with `on`: name one pin and the hole its lead goes into. orbe seats the other leads from the part's shape, including leads off the 2.54 mm grid. `"ldr": { "on": { "a": "bb.c21" } }` puts the LDR's lead `b` in `c22`. `"rotate": [0, 0, 90]` turns the part a quarter turn first. Wire the rest with jumpers between holes. A jumper colour is `"color": "#d12b2b"`.

`at` alone only draws a part. Its leads join no hole. When that `at` puts a leg in a hole and the leg joins nothing there, `orbe lint` warns `scene.unplaced` and names the leg. The hint seats it with `on`, or a contact wire when `on` would claim a hole a wire holds, and it keeps a non-zero `rotate`. A leg already on that hole's net, or one further below the board than the hole is deep, does not warn.

Run `orbe check` and `orbe lint` after seating. A lead in the channel, off the board or in a taken hole is an error. The hint names a fit when one exists. Two pins of one part in one strip warn `scene.shorted`.

A seated part sits on the board's top, and each leg joins its hole's strip. A lead whose tip stays above that top, trimmed on its own module, is not a leg. At no turn, a resistor's legs are 4 columns apart and an LED's are 1, along the row. A leg longer than a hole is deep stands the part up. An LED's 27.4 mm legs hold its body 19.4 mm above the board. `orbe part` prints the hole depth.

A pin on two or more legs goes in by its lowest lead. `orbe part` lists each pin's leads. A switch's `a` is on leads 1 and 2, so `"a": "bb.f22"` puts lead 1 in `f22`. Naming a hole another leg would land in moves the whole part. With `"a": "bb.e22"`, lead 1 goes in `e22`, lead 2 lands in `b22`, and the switch no longer crosses the channel. A label on legs of two parts, such as the Nano's `GND` on two header pins, names no one hole, and `on` refuses it. Name a pin on one leg, such as `D13`.

The core example on a breadboard. The Nano crosses the channel over columns 1 to 15.

```jsonc breadboard.jsonc
{
  "format": 1,
  "parts": {
    "bb": { "type": "breadboard-400" },
    "nano": { "type": "nano-board", "firmware": "build/button.ino.hex" },
    "btn1": { "type": "tactile-switch-6mm" },
    "r2": { "type": "resistor-axial", "resistance": "10k" },
    "r1": { "type": "resistor-axial", "resistance": "220" },
    "led1": { "type": "led-5mm" },
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
breadboard.jsonc: ok, 6 scene parts, expanding to 21 engine parts, 36 nets, ground GND
```

`orbe open breadboard.jsonc` opens it. A path is optional.

Read a hole as a pin, as `orbe get @bb.a12`. `snapshot` lists no holes.

A leg in the channel, off the board, between holes, or in a hole another leg holds, is an error that names the leg and the hole. The hint is a placement that seats every leg with no strip joining two of the part's pins: a quarter turn from the same hole, else a hole within 10.16 mm and a turn, else no quarter turn seats it there. A hole holds one lead. A wire end in a hole a leg or another wire end already names is an error that names both. The hint lists every free hole in that strip, in order, as `a hole holds one lead. free holes in this strip: bb.c1, bb.e1`, or says `a hole holds one lead, and this strip has no free hole`. A wire may still end on a pin off the board, such as an Uno header pin.
