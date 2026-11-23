# orbe: page

Load this when someone watches the 3D page, or a knob turns.

## Watching

`orbe open --headed` opens the scene and a page in the default browser. A person watches it in 3D and clicks its buttons. When a viewer is already connected, it prints the address and opens no browser. `orbe show` opens the page on a session that already runs. `orbe show --no-browser` only prints the address.

Only this machine can open the page. From another, `ssh -L <port>:127.0.0.1:<port>` and open `http://127.0.0.1:<port>/` with the same `#t=` key. The address holds the session's key. Give it only to the user.

Several pages can watch one session. `orbe close` ends them. A click is `press`. Letting go is `release`. While paused, a click changes no pin. The firmware runs only as time moves.

A session opens paused. Run `orbe play` before you hand over the page. Time then runs at real speed until `orbe pause`, a `step` that runs, the 2-hour stop or a fault. It uses the CPU while it runs. `play` reports nothing as time runs. `orbe pause`, or the next `orbe step`, lists what changed while playing, including what the user did on the page.

`show` failing with "this orbe was built without the viewer page" means this `orbe` has no page. Tell the user to rebuild `orbe`.

The page's glow is the mean current since the last frame, so a PWM-dimmed LED glows at its average. With no `layout`, the page puts the parts in a row. Load `orbe skills get breadboards` to seat them.

## A servo

`get @servo.angle` reads the shaft in degrees, and the page turns the horn to it. A pulse outside the servo's range clamps, and when the pulses stop the horn stays where it is.

## A knob

`set` turns a knob while the session runs. It changes only a param `orbe part` says `set` changes, inside its range. The next `step` solves with it, so the wiper's voltage and the sketch's `analogRead` move. `orbe get @pot1.position` reads it.

This wiper feeds A0, and D13 lights over half scale.

```text
open
set @pot1.position 0.8
step 100ms
expect @led1 lit
```

```bash
# turn the knob to 0.8 and let the sketch read it
orbe set @pot1.position 0.8
orbe step 100ms
orbe expect @led1 lit
```
