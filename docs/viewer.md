# The viewer

The viewer draws a scene in 3D from the messages a session sends, and turns a person's clicks into the CLI's own commands. It computes no electrical value.

- `packages/viewer` is the viewer: React Three Fiber, three.js, drei and postprocessing, in TypeScript. Its bench pieces come from maker-sandbox, MIT, Copyright 2026 Alwurts, and each file adapted from it says so in its first line.
- `apps/viewer` is a page that runs the viewer on a fixture of the first target. Vite builds it to static files in `apps/viewer/dist`.

The two are npm workspaces of the repo's root `package.json`, pinned by `package-lock.json`.

## Run it

```bash
npm ci
npm run dev --workspace apps/viewer
```

The page replays the fixture's frames: idle, then `btn1` pressed with D13 high and `led1` lit, then released, one every 2 s. Add `?frame=idle`, `?frame=pressed` or `?frame=released` to the URL to hold one frame. Press `T` to switch between the 3D view and the top view. The browser console shows each command the viewer sends, such as `press @btn1`.

The 3D view orbits with the left button or one finger, pans with the right button, and zooms with the wheel or a pinch. The top view looks straight down, with the part frame's +X to the right and +Y up the screen. It pans with the left or right button or one finger, zooms with the wheel or a pinch, and does not orbit.

The dev server and `vite preview` serve each part's GLB from `parts/` at `/parts/<id>/<id>.glb`. The build leaves the GLBs out.

`scripts/check-viewer.sh` runs the type check, the unit tests and the build. `scripts/check.sh` does not run it, so `check.sh` needs no Node.

`scripts/check-viewer-e2e.sh` builds the page and drives it in Chromium through `apps/viewer/e2e/e2e.mjs`: mouse and touch presses, the right button and both orders of a chord, a cancelled touch and a `pointercancel` while the canvas keeps the capture, lost capture, a second finger on the held button, blur, the press listeners' removal, `touch-action`, a refused or failed send, a press held across a new scene, clicks that send nothing, the top view's axes and pan, hover reasons, the 0603 tints and where bloom starts. It installs no browser. It uses the Chromium that playwright-core expects under `~/.cache/ms-playwright`, or the binary in `ORBE_CHROMIUM`, and exits 2 when it finds neither. `--renders <dir>` also saves screenshots. `check-viewer.sh` does not run it, so `check-viewer.sh` needs no browser.

## The messages

`packages/viewer/src/protocol.ts` defines every message, and its comments give each field's units. A session sends one `scene` message per `open` and a `frame` message after each change. The viewer sends a `Command` back. A `SessionSource` carries them both ways: a WebSocket to the session process, a Worker in the sandbox, or the fixture.

- **`scene`** lists the scene's instances, each with its children to any depth and its placement, or `null` for a part the scene does not place. An instance that is not simulated carries the reason, its own or its nearest marked ancestor's. `parts` holds one entry per part id: the GLB's URL, the anchors, the pins, `view`, `joints`, `controls`, the brightness reading, the colour tints and the part file's `not_simulated` entries. Those entries name features the engine leaves out, such as light or temperature, not a dropped part, and the viewer does not read them yet. `wires` lists every wire with its kind, and `channels` names the refs that each frame array follows. `not_simulated` lists each pin whose net reaches no simulated pin and has a reason, the one `orbe check` gives for a wire to it, such as `@nano.A6`. Each entry carries its `why`, and its `ref` is the one hover shows for the pin.
- **`frame`** holds the pin levels, the LED currents and the switch contacts at `t_ps`, in the order of `channels`. It may also carry the warning count. `speed`, `ratio` and `displays` are reserved until the engine has a play speed and displays.

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

- A primary-button `pointerdown` on a part with `controls.press` sends `press @<ref>` and captures the pointer. Another mouse button sends nothing. That pointer's `pointercancel` or lost capture, its `pointerup`, or the window losing focus, sends `release @<ref>`, once. A mouse press ends once the primary button is up, on a `pointerup` or, in a chord, on the `pointermove` that reports it. Another pointer lifting does not release it, and while one press is held, no other starts.
- A command the session refuses, or whose send fails, is listed in red at the bottom of the canvas and in the console. A release clears the held press even then, and the viewer never resends it. A new `scene` clears the list.
- A new `scene` drops a held press without a release, because it follows an `open`, which restarts the session.
- A click on any other part sends nothing, even when that part holds a pressable child. A click on the Nano's reset button presses `@nano.sw_reset`. A click on the board around it does nothing.
- Hover shows the ref under the pointer. A lead shows the outermost label that reaches it, such as `@nano.D13` on header `j2`'s pin 1, or else its own pin, such as `@led1.anode`. Any other mesh shows its instance, such as `@nano.r_l`.
- Hover also gives the reason for an instance that carries `not_simulated` and for a pin in the scene's `not_simulated`, such as `@nano.A6`'s "The engine's atmega328p kind has the DIP's pins, with no ADC6 or ADC7 pad." A pin takes its reason only from the scene's list, because a lead of a part that is not simulated can still sit on a simulated net. The viewer does not yet draw these parts or pins differently.
