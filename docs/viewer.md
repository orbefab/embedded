# The viewer

The viewer draws a scene in 3D from the messages a session sends, and turns a person's clicks into the CLI's own commands. It computes no electrical value.

- `packages/viewer` is the viewer: React Three Fiber, three.js, drei and postprocessing, in TypeScript. Its bench pieces come from maker-sandbox.
- `apps/viewer` is a page that runs the viewer on a fixture of the first target. Vite builds it to static files in `apps/viewer/dist`.

The two are npm workspaces of the repo's root `package.json`, pinned by `package-lock.json`.

## Run it

```bash
npm ci
npm run dev --workspace apps/viewer
```

The page replays the fixture's frames: idle, then `btn1` pressed with D13 high and `led1` lit, then released, one every 2 s. Add `?frame=idle`, `?frame=pressed` or `?frame=released` to the URL to hold one frame. Press `T` to switch between the 3D view and the top view. The browser console shows each command the viewer sends, such as `press @btn1`.

The dev server and `vite preview` serve each part's GLB from `parts/` at `/parts/<id>/<id>.glb`. The build leaves the GLBs out.

`scripts/check-viewer.sh` runs the type check, the unit tests and the build. `scripts/check.sh` does not run it, so `check.sh` needs no Node.

## The messages

`packages/viewer/src/protocol.ts` defines every message, and its comments give each field's units. A session sends one `scene` message per `open` and a `frame` message after each change. The viewer sends a `Command` back. A `SessionSource` carries them both ways: a WebSocket to the session process, a Worker in the sandbox, or the fixture.

- **`scene`** lists the scene's instances, each with its children to any depth and its placement, or `null` for a part the scene does not place. `parts` holds one entry per part id: the GLB's URL, the anchors, the pins, `view`, `joints`, `controls`, the brightness reading and the colour tints. `wires` lists every wire with its kind, and `channels` names the refs that each frame array follows.
- **`frame`** holds the pin levels, the LED currents and the switch contacts at `t_ps`, in the order of `channels`.

Lengths, positions and directions in the messages are in the part frame: mm, +Z up, with the seating plane at z = 0.

## How it draws a scene

- Every instance is a group named by its ref, such as `@nano.j2`, placed by its parent's layout in the parent's part frame. A part with children draws its own GLB, if it has one, and then each child's.
- One turn at the root, `TO_THREE` in `build.ts`, maps the part frame into the viewer's world, which is mm with +Y up. Part (x, y, z) lands at world (x, z, -y).
- The viewer loads each GLB URL once. It refuses a GLB with more than one root node, or whose root carries anything other than glTF's -90 degree turn about X. It removes that turn and scales the GLB by 1000, so the GLB's nodes sit in the part frame in mm.
- An LED's `view.glow` node glows by its current over `readings.brightness.full`. Intensity 1 is the bloom threshold.
- A `view.tint` node takes the colour of the choice its param picks. A `view.recolour` node set with rule `iec60062-4-band` shows the resistance in the 4-band colour code.
- The joint that `controls.press.joint` names slides its node by `range` along `slide` while the frame says the switch is closed.
- `jumper` wires are drawn. `trace` and `contact` wires are not. Each end sits on a lead's anchor. A pin on two leads, such as a switch's `a` or the Nano's `GND`, uses the lead nearest the other end.
- The viewer lays out each unplaced part in a row to the right of the placed ones. It writes nothing back.

## Clicks and hover

- `pointerdown` on a part with `controls.press` sends `press @<ref>` and captures the pointer. `pointerup`, `pointercancel`, losing the capture and the window losing focus each send `release @<ref>`, once.
- A click on any other part sends nothing, even when that part holds a pressable child. A click on the Nano's reset button presses `@nano.sw_reset`. A click on the board around it does nothing.
- Hover shows the ref under the pointer. A lead shows the outermost label that reaches it, such as `@nano.D13` on header `j2`'s pin 1, or else its own pin, such as `@led1.anode`. Any other mesh shows its instance, such as `@nano.r_l`.
