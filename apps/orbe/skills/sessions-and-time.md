# orbe: sessions and time

Load this when a step or a set lands at the wrong time, a session's name or stop matters, or a switch bounces. `orbe skills get core` has the commands.

## Time

- `orbe step <duration>` takes `ps`, `ns`, `us`, `ms` or `s`, such as `100ms` or `20us`. Serial edges on D1 can fill the 20 lines `step` prints.
- `step` lists pin edges with their times, LED changes and diagnostics. It lists at most 20, for what `play` held and again for what the step did, then `and N more`, then every LED's `brightness`.
- An input lands at the session's time or later. The reply gives that time. A world `set` also lands after the last change the scene's `steps` schedule, so it never undoes one. A part's `set`, such as `@pot1.position`, lands at once. Load `orbe skills get page` to turn a knob.
- `step --until` reads the condition now. If it already holds, it takes no time. A pin condition stops 1 ps after the edge. `@led1.i` shows the current there. The light state follows the 20 ms average. Other conditions are read every 1 ms, so the reply says `by 104ms`.
- Serial can arrive up to one engine step (`sim.max_step`, 10 ms by default) after the chip sends it. Step past a print before you expect it.
- A session stops at 2 hours. Reads still work. `press`, `release`, `set`, `step` and `play` fail with `ended`. `step <duration>` past the stop fails with `past_stop`, and the hint names the time left. `step --until` stops there and fails with `ended`. An input that would land past the stop fails with `past_stop`. The hint says to run `orbe open`.

## Sessions

`orbe open` starts a background process. Later commands talk to it. It ends on `orbe close`, or after 1 hour with no command while it is not playing and no page watches it.

The session name comes from the project folder, the nearest one up with `scene.jsonc` or `parts/`. Each project gets its own, and its `tests/` shares it. Outside a project it comes from the git repository's root, else the current folder. `--session <name>`, or `ORBE_SESSION=<name>`, picks another.

`orbe close --all` closes every session and cleans up after crashed ones. `session_crashed` names `<name>.log.crashed` when the process left output. A failed `open` after an earlier crash names `<name>.log`, the retry's output. A crash that wrote nothing removes it.

A command other than `close` refuses a session another orbe version started, as `version_mismatch`. Run `orbe close`.

A command sent while a `step` runs waits for it. Ctrl-C during a long `step` stops it. The session stays paused at the time it reached, and the error is `cancelled`.

With `--json`, a live command adds `session`, `t_ps` in picoseconds, and `state`: `paused`, `playing`, `stopped`, `faulted` or `closed`.

## Bounce

A switch with `bounce` set bounces up to `max_bounces` times inside that window after each `press` and `release`. The next input lands after the window of the one before. With this change to the core scene,

```diff scene.jsonc
-    "btn1": { "type": "tactile-switch-6mm" },
+    "btn1": { "type": "tactile-switch-6mm", "bounce": "2ms" },
```

a `release` and a `press` with no `step` between land 2 ms apart. `step --until @led1 lit` stops once the average crosses 2 mA, after the first contact has opened again inside the window.

```console
$ orbe open
opened scene.jsonc at t=0: nano, btn1, r2, r1, led1 (5 scene parts, expanding to 20 engine parts, 35 nets)
...
$ orbe step 100ms
t=100ms (from 0s)
...
$ orbe press @btn1
pressed btn1 at t=100ms; reads show it after t=100ms
$ orbe step --until @led1 lit
t=105ms (from 100ms), @led1 lit by 105ms
...
$ orbe release @btn1
released btn1 at t=105ms; reads show it after t=105ms
$ orbe press @btn1
pressed btn1 at t=107.000002ms; reads show it after t=107.000002ms
$ orbe step 6ms
...
$ orbe close
closed at t=111ms
```

To read the switch settled, step past the landed time in the last input's reply plus the window. Here that is a `step 6ms` from 105 ms.
