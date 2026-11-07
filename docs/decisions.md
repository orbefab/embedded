# Decisions

Investigations may challenge any of these, with reasons.

## Firmware toolchains are not ours (Oct 31 2026)
The engine runs firmware built with any toolchain, and users bring whatever they like. Orbefab never builds its own compiler. The skills recommend existing tools: arduino-cli for AVR/Arduino, ESP-IDF for ESP32.

## Entry points (Nov 1 2026)
- The CLI is `orbe`. If it ships through npm, where `orbe` is taken, a scoped package can still install an `orbe` binary.
- A background process per session holds the simulation, so short commands share state. Headless is the normal case for agents and CI. Headed opens the viewer on the same session, and the person can interact too.
- The simulation pauses between commands by default, and plays, pauses or steps when told. Sessions end after 2 hours.
- Files are the record: the scene, part files, tests and the paths of firmware the user built. The live session is the agent's working loop.
- Scenes are data files. No Python scenes at first.
- Tests are command files, `tests/*.test`, with `expect` lines.
- Output is plain text, or JSON with `--json`.
- A recording is for replay and checking only. To continue, rerun the commands on the scene.
- The ESP32-S3 gets its time from a scheduled `console` file the scene names. There is no live send command.
- Skills work with every agent: a thin stub, installed with skills.sh, points at the CLI, which serves the full skill for its own version.
- No MCP server for now. One comes only if a use appears.
- No legacy. Nothing is kept for compatibility with old scenes.

## No hidden actions (Nov 4 2026)
A command does what it names and nothing in the background. Where `orbe` could guess or quietly fix something, it stops with a clear error that says what happened and what to run instead. A reply carries what concerns its own command (its result, its errors and how to fix them), and nothing unrelated is added to every reply.

## Parts (Nov 4 2026)
- A part is one real physical object, defined in one file, `part.jsonc`. Every field is optional except `format`. A part may have an engine model, children (`parts`, `wires`, `layout`), both or neither.
- A part with no model that only joins the wires that reach its pins, such as a pin header, says so with `"joins_only": true`. `orbe lint` then asks it for no model.
- A scene is not a part. We ship parts only.
- A param is a number, a choice or a file. A choice picks a named set of values, such as an LED's colour, and may tint the 3D model. A param never changes pins or shape. An LED's colour is required.
- A part with children passes a param to a child with `{ "param": "<name>" }` on the child, as a prop passes through a component. The whole value passes, with no arithmetic for now.
- Catalog parts name the real object in `real` (maker, part number, datasheet), and every model value cites its source.
- A part's files, such as the ESP32-S3 ROM, live in its folder. `files` maps a name to a path.
- A part states its readings, such as an LED's `on` above 1 mA.
- `joints` say what moves, such as a button's plunger, and `controls.press` names the joint it drives. Joints between two parts come later.
- Versions are three numbers. A patch or minor version can't change a passing test.
- Parts that share an engine model, such as two packages of one chip, each hold a copy of its values. A check keeps the copies equal.

## Catalog and projects (Nov 4 2026)
- The catalog is a registry: a static index plus one archive per part version, shaped like shadcn's registry. `orbe` reads it itself.
- `orbe add <id>…` copies catalog parts, and the parts they use, into the project's `parts/<id>/`. A project holds one version per id. `orbe add --diff` shows what a newer version changes, and `--overwrite` takes it. Crossing a major version needs the version named.
- Only `orbe add` copies or fetches parts. `orbe open` and `orbe test` never do. A missing part is an error that gives the exact `orbe add` command.
- `parts.lock` records each catalog copy: its id, version, registry and hash. It is a checksum record, with no version ranges, no solver and no shared store. A fresh clone holds every part and can check them offline.
- Catalog parts are sealed, except a part built only from other parts. A changed sealed copy is an error. Restore it with `orbe add <id> --overwrite`, or copy it to a part of your own. A catalog part built from other parts, such as the nano board, can be edited, and checks report it as modified.
- Parts from other registries are named `@<registry>/<id>`. The first release reads the official registry only.
- 3D models live in plain git, in the part's folder.

## Scenes (Nov 4 2026)
- A scene is its own kind of file, usually `scene.jsonc`. Its keys are `format`, `parts`, `wires`, `layout`, `sim` and `nets`. It has no `pins` or `params`, and no file can use a scene as a part.
- Scenes and part files are JSONC: JSON plus `//` comments and trailing commas, nothing more. `orbe` generates a JSON Schema, `parts/schema.json`, from the project's parts. A file's `$schema` key points editors at it for completion and value checks.
- An instance has a `type` (a catalog id, or a `./` path to a part of your own), its params, and an optional `not_simulated` reason.
- `wires` lists pin pairs, such as `["nano.D2", "btn1.a"]`, and `orbe` works out the nets. Every wire has a kind. `jumper` is the default and is drawn. `trace` (copper on a board) and `contact` (two pins touching, such as a leg in a breadboard hole) are not drawn. Drawing traces comes later.
- `layout` sits in the parent, keyed by name, with `{ "at": [x, y, z], "rotate": [rx, ry, rz] }` in mm and degrees. The rotation is about the parent's fixed X, then Y, then Z, URDF's order. `mate` is reserved for later.
- A net takes the name of its smallest label. A scene can name nets in `nets`, such as `"GND": "nano.GND"`.
- Nested names flatten for the engine by joining with `__`, so `@nano.D13` is the net `nano__D13`. Names may not contain `__`.
- A viewer edit goes to the file that holds what it changes. If that file is used more than once, the viewer asks first. Sealed parts refuse edits. A part with no position is placed by the viewer for drawing only, and nothing is written until a person drags it.
- Two hashes: one over every project file read, which guards writes, and one over the netlist, which says whether the simulation is current.
- The ESP32-S3 takes one file param per flash image, `partitions` and `app`. Its ROM comes from the chip part's files.

## Checking (Nov 4 2026)
Type checking, linting and runtime diagnostics are separate.
- `orbe check` type-checks: do the files describe a valid circuit? Every finding is an error. `orbe open` runs it first.
- `orbe lint` runs `orbe check`, then lint rules that never block, such as an unnamed ground net, a part with no position or an unconnected pin. It includes the engine's static checks, such as a floating net.
- A part that holds both a source and a chip, such as a board, runs with nothing wired to it, so lint does not call it unconnected. The rule reads the engine model kinds, not part names.
- Runtime diagnostics come only from `run`, `step` and `test`.
- `orbe open` shows type errors and the engine's static warnings.

## The nano board (Nov 4 2026)
- It is built from 14 leaf parts, 28 children in all, wired with traces. The viewer draws it by composing its children's 3D models, with no merged model.
- Its header children sit at z = 0 with their plastic under the board, so it seats on a breadboard.
- A6 and A7 are not simulated yet, since the engine's 328P has the DIP's pins. A wire to them is an error that says so.
- Its 16 MHz clock is a three-terminal ceramic resonator, until a clone's photo says otherwise.

## The uno board (Nov 7 2026)
- It is built from 25 leaf parts, 49 children in all. Its traces are the copper of Arduino's UNO-TH_Rev3e board file, net by net.
- It runs from USB, as the Nano does. Its 3.3 V regulator runs on the engine's LP2985 fit, so 3V3 works. VIN, the barrel jack, the 5 V regulator and the comparator that switches USB off are not simulated, and a wire to VIN is an error that says so.
- The engine has no op-amp, so a wire stands in for the follower between D13 and the L LED, as the engine's board example drives that LED from D13. D13 with its pull-up on then reads low, where the board reads high.
- The engine has no P-channel MOSFET, so the USB switch is a resistor at its on resistance, always on, as in the engine's board example.
- A part is simulated whole. RN1's sections divide VIN, so RN1 is out, and its RESET pull-up with it. The 328P's own pull-up holds RESET high.
- AVCC is on the 5V rail, as the revision 3 schematic draws it. L2, the inductor the Rev3e board file puts between them, is drawn but not simulated. The engine's 328P draws nothing from AVCC, and with L2 in place a step took about a thousand times as long.
- C4 on AREF starts where the circuit's DC solution puts it, with no held voltage. A 0 V hold would pull a rail wired to AREF to 0 V at t = 0. AREF with nothing wired to it is then a floating net, as on the Nano.

## Later
Mates between parts, joints between two parts, importing outside CAD, PCB routing, drawing traces, per-instance placement overrides, A6 and A7, and an `orbe lsp` for editors.

## Repo (Oct 31 2026)
One private repo for the whole branch: `orbefab/embedded`. The current sim-engine code moves in first, then maker-sandbox, then the rest (Blocks, skills, examples). The sandbox then builds against the current engine instead of a frozen copy.

## Parts model (Oct 31 2026)
Every part (circuit parts, sensors, dev boards) should eventually have its sim model, specs and 3D model, organized together. An inventory of what exists is the next step.

## 3D models (Oct 31 2026)
`orbefab/models` is where 3D models are built. The finished models a part needs end up living in this repo with that part.

## Naming and nesting (Oct 31 2026)
- Parts use generic names (e.g. `uno-board`, `led-5mm`), not brand or product names.
- Everything is a part, and parts can nest.
- ICs (e.g. the 328P) are black boxes: the smallest unit, never broken down further.
- A board is a part built from other parts. To change one, you edit your project's copy of it (see "Catalog and projects").
- Prior art: sim-all-proto ADR 0010 (parts nest) and the layered-sim Uno/Nano composites. Today's sim-engine board example is flat.

## Complete-part order (Nov 2 2026)
The first boards taken to complete (sim, specs, 3D):
1. The nano board, for the first demo (a button and an LED).
2. The uno board.
3. The knob (Guition) as its own separate part, which also happens to use an ESP32-S3. It replaces the plain ESP32-S3 dev board in the order.
4. Then others.

The ESP32-C3 and the original ESP32 come later (sim-engine models only the S3 today).

## Licensing (Oct 31 2026)
- The code is under MIT OR Apache-2.0, at the user's option (Nov 2 2026), the same as `orbefab/sim-engine`.
- Part files here (sim models, specs, 3D) use the same license as the code.
- A third-party file a part carries, such as the ESP32-S3 ROM, keeps its own license. It ships with the upstream license files, unchanged, beside it.
- Our own models in `orbefab/models` are Orbefab copyright, so they can be relicensed.
- Third-party CAD keeps its own license (SO-ARM100 Apache-2.0, Adafruit MIT).
- The Waveshare Bus Servo Adapter model has no license and can't go public; it would need our own remodel.

## Parts catalog timing (Oct 31 2026)
The old trigger (a second view, a second shell or ~15 parts) is met, since both web and local visualizers are planned. The catalog is designed in the parts-organization investigation and filled as the engine moves in, starting with the nano board.

## Visualizer shells (Oct 31 2026, framework Nov 2 2026)
A web visualizer (the sandbox) and a local-computer visualizer share one viewer: three.js with React Three Fiber, reusing maker-sandbox's bench.
- Locally it's a browser tab that the session process serves, with the page built into the CLI. Nothing else installs.
- In the sandbox the same page runs the engine's session loop in a Web Worker, compiled to WebAssembly.
- The viewer's world is Y-up in mm. Parts are authored in their own frame, mm with +Z up, and one mapping converts between the two.
- 3D models are GLBs exported by each `orbefab/models` script, with part names kept and a pin-anchor file beside them.
- Whole-scene screenshots use an open viewer, or else a Chrome, Chromium or Edge already on the machine.
- The viewer sends input commands only. It can't restart a session. Its edits go to the file that holds what they change (see "Scenes") and show as not simulated until the next open.

## No 2D views (Nov 2 2026)
Parts have no 2D schematic view. The top-down view is a camera on the 3D scene.

## Engine naming (Nov 1 2026)
Inside this repo the simulator is just "engine" (perhaps; may change if it clashes with a visualization engine). `orbefab/sim-engine` isn't renamed; it's archived once its code has moved.
