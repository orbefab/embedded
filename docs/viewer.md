# The viewer

The viewer draws a scene in 3D from the messages a session sends, and turns a person's clicks into the CLI's own commands. It computes no electrical value.

- `packages/viewer` is the viewer: React Three Fiber, three.js, drei and postprocessing, in TypeScript. Its bench pieces come from maker-sandbox, MIT, Copyright 2026 Alwurts, and each file adapted from it says so in its first line.
- `apps/viewer` is the page. On a live session it runs the viewer on the session's WebSocket, and on its own it runs the viewer on a fixture of the first target. Vite builds it to static files in `apps/viewer/dist`.

The two are packages in the repo's pnpm workspace, pinned by `pnpm-lock.yaml`.

## Run it

```bash
pnpm install --frozen-lockfile
pnpm --filter @orbe/viewer-page dev
```

The page replays the fixture's frames: idle, then `btn1` pressed with D13 high and `led1` lit, then released, one every 2 s. Add `?frame=idle`, `?frame=pressed` or `?frame=released` to the URL to hold one frame. Press `T` to switch between the 3D view and the top view. The browser console shows each command the viewer sends, such as `press @btn1`.

The 3D view orbits with the left button or one finger, pans with the right button, and zooms with the wheel or a pinch. The top view looks straight down, with the part frame's +X to the right and +Y up the screen. It pans with the left or right button or one finger, zooms with the wheel or a pinch, and does not orbit.

The dev server and `vite preview` serve each part's GLB from `parts/` at `/parts/<id>/<id>.glb`. The build leaves the GLBs out.

`scripts/check-viewer.sh` runs the type check, the unit tests and the build. `scripts/check.sh` does not run it, so `check.sh` needs no Node.

`scripts/check-viewer-e2e.sh` builds the page and drives it in Chromium through `apps/viewer/e2e/e2e.mjs`: mouse and touch presses, a finger moving while it holds a press, the right button and both orders of a chord, a cancelled touch and a `pointercancel` while the canvas keeps the capture, lost capture, a second finger on the held button, blur, the press listeners' removal, `touch-action`, a refused or failed send, a press held across a new scene, clicks that send nothing, the top view's axes and pan, hover reasons, the 0603 tints and where bloom starts. It installs no browser. It uses the Chromium that playwright-core expects under `~/.cache/ms-playwright`, or the binary in `ORBE_CHROMIUM`, and exits 2 when it finds neither. `--renders <dir>` also saves screenshots. `check-viewer.sh` does not run it, so `check-viewer.sh` needs no browser.

## Watch a live session

`orbe show` opens the page on a session that runs, and `orbe open --headed` is `orbe open` and then `orbe show`. When that `show` fails, the command fails with `show`'s error and exit code, the session stays open, and the hint says to run `orbe show` again or `orbe close`. The page is built into `orbe`, so build it before `orbe`:

```bash
pnpm install --frozen-lockfile
pnpm --filter @orbe/viewer-page build
cargo build
cd examples/first-target
../../target/debug/orbe open --headed
```

`orbe`'s build embeds `apps/viewer/dist` when it holds a page. Without one, `orbe` still builds, and `show` fails with `io_error` "this orbe was built without the viewer page" and a hint with these commands. `ORBE_VIEWER_DIR=<folder>` serves a built page's folder in place of the embedded one.

- The first `show` makes the session's process listen on 127.0.0.1, on a port the system picks, with a fresh 256-bit token. Each later `show` gives the same URL, `http://127.0.0.1:<port>/#t=<token>`. The token sits in the fragment, so the browser never sends it in a request for the page.
- `show` opens the URL in the default browser with `xdg-open`, or `open` on macOS. `--no-browser`, on `show` or with `open --headed`, prints the URL and opens no browser. Tests and scripts always pass it.
- Every request must carry `Host: 127.0.0.1:<port>`, which turns away DNS rebinding. The WebSocket at `/session?t=<token>` also needs the token and `Origin: http://127.0.0.1:<port>`, checked once on the upgrade. Anything else gets 403. The process logs no request.
- The server answers `GET` and `HEAD` with the page's files, and `/parts/<id>/<id>.glb` with each part model the scene message names, from the project's `parts/` folder. It serves no other file. It serves at most 32 connections at once, pages and their requests together, and closes any more at once.
- Several pages can watch one session. Closing a page leaves the session running. `orbe close` sends each page `closed` with the reason, then closes its socket. A page whose socket drops without one says the connection dropped. The process also ends after an hour with no command, but that hour counts only while no page watches and `play` is not running, so it never ends a page's session or a session that plays.

One thread owns the session. CLI commands, the pages' commands and `play`'s ticks reach it on one queue, so a page's `press` runs between two commands, never inside a `step`, and lands at `now` like a CLI `press`. A page sends only `press`, `release`, `play` and `pause`, each parsed by the CLI's own command table. The page shows the state, the time and a Play/Pause button, lists a refused command as the CLI prints its error, and says why the session ended.

- A frame reads the probe, the pin levels and the switch contacts at `now`, and never lands a step, so a session gives the same replies with a page watching or without one. The session sends a frame after each command that changes what a frame shows, and during a long `step` at most one every 16 ms of wall time.
- `orbe play` advances time in ticks of 16 ms of wall time, each by the wall time since the last tick, up to 16 ms, at speed 1. An engine slower than real time plays in slow motion. A tick's frame carries `speed` 1 and `ratio`, the simulated time over the wall time of that tick. A `step` that runs, `pause`, the 2-hour stop or a fault ends play. A `step` refused before it runs, such as `past_stop`, leaves play running. The changes the ticks ran wait for the next `step`, which lists them before its own and gives `from` as where the `step` before it ended. A `step` refused at the stop or a fault lists them in its refusal. A test file refuses `play` and `pause`.

`scripts/check-headed.sh` builds the page and `orbe`, then drives a live session of the first target in Chromium through `apps/viewer/e2e/headed.mjs`. It runs `orbe open --headed --no-browser` in a runtime folder of its own, plays, holds `@btn1` until `@led1` lights, and checks that `orbe get @led1` reads the current the viewer glows at. It closes the session however it ends. It finds Chromium as `check-viewer-e2e.sh` does, and `--renders <dir>` saves screenshots.

## The messages

`packages/viewer/src/protocol.ts` defines every message, and its comments give each field's units. A session sends one `scene` message per `open` and a `frame` message after each change. The viewer sends a `Command` back. A `SessionSource` carries them both ways: `websocketSource` to the session process, a Worker in the sandbox, or the fixture. The page uses `websocketSource` when its URL's fragment holds `t=<token>`, and the fixture otherwise. `window.orbeFixture` exists only on the fixture page.

- **`scene`** lists the scene's instances, each with its children to any depth and its placement, or `null` for a part the scene does not place. An instance that is not simulated carries the reason, its own or its nearest marked ancestor's. `parts` holds one entry per part id: the GLB's URL, the anchors, the pins, `view`, `joints`, `controls`, the brightness reading, the colour tints and the part file's `not_simulated` entries. Those entries name features the engine leaves out, such as light or temperature, not a dropped part, and the viewer does not read them yet. `wires` lists every wire with its kind, and `channels` names the refs that each frame array follows. `not_simulated` lists each pin whose net reaches no simulated pin and has a reason, the one `orbe check` gives for a wire to it, such as `@nano.A6`. Each entry carries its `why`, and its `ref` is the one hover shows for the pin.
- **`frame`** holds the pin levels, the LED currents and the switch contacts at `t_ps`, in the order of `channels`. An LED's current is its mean since the frame before. A page joining restarts that mean, so every page then gets a frame with each LED's present current, the one `orbe get` reads. It also carries the warning count. Its `state` is the session's, or `stepping` on a frame sent while a `step` runs, which no reply says. A frame from a `play` tick carries `speed` and `ratio`. `displays` is reserved until the session sends its displays.
- **Over the WebSocket**, the viewer sends each command with an `id` of its own, as `{"id": 1, "cmd": "press", "ref": "@btn1"}`. The session answers that page alone with `{"type": "reply", "id": 1, "reply": ...}`, where `reply` is the envelope `orbe press @btn1 --json` prints: `ok`, `session`, `t_ps`, `state`, then `data` or `error`. The last message is `{"type": "closed", "reason": ...}`.
- The session's channels are the board pins `snapshot` shows that a chip pin drives and a wire reaches, every LED whose part data has a brightness reading, and every switch with `controls.press`.

Lengths, positions and directions in the messages are in the part frame: mm, +Z up, with the seating plane at z = 0.

## How it draws a scene

- Every instance is a group named by its ref, such as `@nano.j2`, placed by its parent's layout in the parent's part frame. A part with children draws its own GLB, if it has one, and then each child's.
- One turn at the root, `TO_THREE` in `build.ts`, maps the part frame into the viewer's world, which is mm with +Y up. Part (x, y, z) lands at world (x, z, -y).
- The viewer loads each GLB URL once. It refuses a GLB with more than one root node, or whose root carries anything other than glTF's -90 degree turn about X. It removes that turn and scales the GLB by 1000, so the GLB's nodes sit in the part frame in mm.
- An LED's `view.glow` node glows by its current over `readings.brightness.full`, with no upper clip. The glow colour is the node's colour scaled to luminance 1, and the lit surface under it fades to black as the intensity goes from 0 to 1. So at the full current the node's luminance is 1, the bloom threshold, and bloom starts there.
- A `view.tint` node takes the colour of the choice its param picks. A `view.recolour` node set with rule `iec60062-4-band` shows the resistance in the 4-band colour code.
- The joint that `controls.press.joint` names slides its node by `range` along `slide` while the frame says the switch is closed.
- `jumper` wires are drawn. `trace` and `contact` wires are not. Each end sits on a lead's anchor. A pin on two leads, such as a switch's `a` or the Nano's `GND`, uses the lead nearest the other end.
- The viewer lays out each unplaced part in a row to the right of the placed ones. It writes nothing back.
- A part the viewer cannot draw as its data says, such as one whose `view` names a node its GLB lacks, shows its GLB as loaded, with no tint, glow or press. It is listed in red at the bottom of the canvas and in the console, and its hover gives the reason. The rest of the scene still draws. A wire that reaches no lead is listed the same way. A resistance with no 4-band code, outside 0.1 ohm to 99 Gohm, keeps the GLB's band colours, with one console warning.
- A GLB whose root node has a mesh is refused, because the viewer draws only the root's children. A child with no placement is refused, with its ref.

## Clicks and hover

- A primary-button `pointerdown` on a part with `controls.press` sends `press @<ref>` and captures the pointer. Another mouse button sends nothing. That pointer's `pointercancel` or lost capture, its `pointerup`, or the window losing focus, sends `release @<ref>`, once. A press ends once the pointer's primary button (a mouse's left button, a pen's tip, a finger's contact) is up, on a `pointerup` or, in a chord, on the `pointermove` that reports it. Another pointer lifting does not release it, and while one press is held, no other starts.
- A command the session refuses, or whose send fails, is listed in red at the bottom of the canvas and in the console. A release clears the held press even then, and the viewer never resends it. A new `scene` clears the list.
- A new `scene` drops a held press without a release, because it follows an `open`, which restarts the session.
- A click on any other part sends nothing, even when that part holds a pressable child. A click on the Nano's reset button presses `@nano.sw_reset`. A click on the board around it does nothing.
- Hover shows the ref under the pointer. A lead shows the outermost label that reaches it, such as `@nano.D13` on header `j2`'s pin 1, or else its own pin, such as `@led1.anode`. Any other mesh shows its instance, such as `@nano.r_l`.
- Hover also gives the reason for an instance that carries `not_simulated` and for a pin in the scene's `not_simulated`, such as `@nano.A6`'s "The engine's atmega328p kind has the DIP's pins, with no ADC6 or ADC7 pad." A pin takes its reason only from the scene's list, because a lead of a part that is not simulated can still sit on a simulated net. The viewer does not yet draw these parts or pins differently.
