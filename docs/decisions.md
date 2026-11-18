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

## Flags follow the command (Nov 8 2026)
`orbe` refuses a flag placed before the command name, as in `orbe --session x open`. An agent learns one way to write a command. The error names the flag and shows those words with the flag after the command name. `--help` and `--version`, and their short forms, still come first. Clap's `global = true`, which would accept the flag in either place, is the alternative not taken.

## Parts (Nov 4 2026)
- A part is one real physical object, defined in one file, `part.jsonc`. Every field is optional except `format`. A part may have an engine model, children (`parts`, `wires`, `layout`), both or neither.
- A part's `format` covers its `model`. A netlist change that an existing part file cannot survive raises the part `format`.
- A part with no model that only joins the wires that reach its pins, such as a pin header, says so with `"joins_only": true`. `orbe lint` then asks it for no model.
- A scene is not a part. We ship parts only.
- A param is a number, a choice or a file. A choice picks a named set of values, such as an LED's colour, and may tint the 3D model. A param never changes pins or shape. An LED's colour is required.
- A part with children passes a param to a child with `{ "param": "<name>" }` on the child, as a prop passes through a component. The whole value passes, with no arithmetic for now.
- Catalog parts name the real object in `real` (maker, part number, datasheet), and every model value cites its source.
- A part's files, such as the ESP32-S3 ROM, live in its folder. A file param's `default` names one, as a path relative to the part file.
- A part states its readings, such as an LED's `lit` above 2 mA and `dim` above 100 uA. `brightness` is the average forward current over the last 20 ms, the time since t = 0 when the session is younger. 2 mA is the lowest current of the forward fit. 100 uA is a hundredth of the 10 mA at which the datasheet rates luminous intensity, and ten times the 10 uA reverse-current maximum.
- `joints` say what moves, such as a button's plunger, and `controls.press` names the joint it drives. Joints between two parts come later.
- Versions are three numbers. A patch or minor version can't change a passing test.
- Parts that share an engine model, such as two packages of one chip, each hold a copy of its values. A check keeps the copies equal.

## Catalog and projects (Nov 4 2026)
- The catalog is a registry: a static index plus one archive per part version, shaped like shadcn's registry. `orbe` reads it itself.
- `orbe add <id>…` copies catalog parts, and the parts they use, into the project's `parts/<id>/`. A project holds one version per id. `orbe add --diff` shows what a newer version changes, and `--overwrite` takes it. Crossing a major version needs the version named.
- Only `orbe add` copies or fetches parts. `orbe open` and `orbe test` never do. A missing part is an error that gives the exact `orbe add` command. When that add would stop because a part it uses differs, the command is the one that add prints.
- `parts.lock` records each catalog copy: its id, version, registry and hash. The hash covers the files the part publishes. Build output in the folder is not one of them. It is a checksum record, with no version ranges, no solver and no shared store. A fresh clone holds every part and can check them offline.
- Catalog parts are sealed, except a part built only from other parts. A sealed copy that differs from its `parts.lock` line is `part_modified`. Restore it with `orbe add <id> --overwrite`, or copy it to a part of your own. A copy that matches its lock line and is an older version than the catalog in this `orbe` is `part_outdated`, and check, open and test stop: this `orbe` carries one version of each id, and a run reads that version. One hint names every outdated id. A copy that matches its lock line and is a newer version is `part_newer`. Its hint says the project needs a newer orbe and names no `orbe add`. `--overwrite` leaves that copy, including a child of a board the command writes, and says the project needs a newer orbe for it. A refusal says the same for each copy it left, and the `orbe add` in its hint names every id that command would write. That command always names an id. When nothing can be written, the hint names no `orbe add` and says the project needs a newer orbe. A line from another registry stays `part_modified` and names that registry. A catalog part built from other parts, such as the nano board, can be edited at the version its lock names, and checks report that edit. An older version of it is still `part_outdated`, and a newer one is `part_newer`. A part whose bytes change without a new version is refused by a registry build. A binary built from that source reports `part_modified` for a sealed copy that locked the previous bytes of the same version.
- Parts from other registries are named `@<registry>/<id>`. The first release reads the official registry only.
- 3D models live in plain git, in the part's folder.

## Blocks (Nov 8 2026)
- A Block is a small circuit that works: a board, parts, firmware and tests. `blocks/<id>/` holds `block.jsonc`, `scene.jsonc`, `firmware/`, `tests/` and `README.md`. It replaces `examples/`.
- `block.jsonc` holds what a Block's catalog card needs from the Block itself: `block` (its id, the folder's name), `version`, `title`, `summary`, `board`, `tags` and `license`. It does not list parts, since the scene names them.
- A Block commits no part copies and no `parts.lock`. A folder that holds `block.jsonc` is a Block, and the reader resolves each catalog id its scene names from the catalog built into the same `orbe`. A Block in this repo then always runs the parts of the commit it sits in, so a part change that breaks a Block fails that commit's checks, and no lock or copies go stale beside the catalog.
- A Block is self-contained. Its firmware sits in its own `firmware/` with the sketch it was built from and the licences that image carries, and its scene names it as `firmware/<name>.hex`, so the path works in the Block and after it is installed.
- `orbe add <block>` installs a Block into a project. It copies the parts the scene names into `parts/` and records them in `parts.lock`, as `orbe add <part>` does, then writes `scene.jsonc`, `firmware/`, `tests/` and `README.md`. `block.jsonc` stays behind, so the installed project is not itself a Block.
- `orbe add` never merges a Block into a scene. A project that already holds a different `scene.jsonc`, or any other file the Block brings, is refused before anything is written, and the hint installs into an empty folder. A file already there with the Block's bytes is kept, so the same install run again finishes or changes nothing.
- Parts and Blocks share `orbe add`, so a Block id is never a catalog part id. A test fails when one is both, and one word names one of them.
- A Block has a version, like a part. `orbe add <block>` installs the one version built into `orbe`. Naming a version is refused, and the hint is that command with the same `-C`.
- A Block's registry record lists every part its scene runs, at the versions in the same build. The build does not run the Block's tests. In this repo `scripts/check.sh` runs them against the parts of the same commit.

## Registry (Nov 8 2026)
- `orbe registry build <src> <out>` builds a registry from any folder of `registry.jsonc`, `parts/` and `blocks/`, so another registry is built with the same `orbe` its users run. The code is the `orbe-registry` library, and the cards come from `orbe-scene`, the same as `orbe part` prints.
- `registry.jsonc` names the registry, such as `@orbefab`, and its homepage, and the index carries both, so an index says which registry it is wherever it is mirrored or merged.
- Item types are `part` and `block`, with shadcn's field names.
- Every record sits at `<id>@<version>` and never changes once served, and its files sit at `files/<sha256>`. Hash dedupe is the only optimisation. A record has no `usedBy`, since that grows with the registry.
- A registry keeps every version it ever served. `--previous` carries an earlier build's records forward and refuses a version whose record would change.
- `released.lock` records the SHA-256 of the served record of each part and Block at the version a release served. It is empty until the first tag. A line stays after its folder is removed. `scripts/check.sh fast` fails a record that differs from its line at that version, and `scripts/released.sh` rewrites the file at a tag, keeping those lines and never moving a version backwards.
- The index and every record carry `"format": 1`. Clients ignore fields they do not know, and removing or changing a field raises the format, so the schemas allow fields they do not name.
- A field with no value is left out, never `null`.
- A dependency is `[@<registry>/]<id>@<version>`, though the build writes only bare ones today, so a parser written now reads cross-registry dependencies later.
- The URL template has one `{name}`, in its last path segment. A client fills it only with `index` and takes every other URL from the index and the records.
- Records stay camel case, as shadcn's are, and `orbe part --json` stays snake case. [registry.md](registry.md) gives the exact mapping and a test converts every catalog part both ways.
- `parts.lock` keeps one line per part id, so a project takes each id from one registry. Its registry field is the registry's name, such as `@orbefab`, not a URL, so a registry that moves changes one line of config and no lock. [registry.md](registry.md) is the client contract.

## Scenes (Nov 4 2026)
- A scene is its own kind of file, usually `scene.jsonc`. Its keys are `format`, `parts`, `wires`, `layout`, `sim` and `nets`. It has no `pins` or `params`, and no file can use a scene as a part.
- Scenes and part files are JSONC: JSON plus `//` comments and trailing commas, nothing more. `orbe` generates a JSON Schema, `parts/schema.json`, from the project's parts. A file's `$schema` key points editors at it for completion and value checks.
- An instance has a `type` (a catalog id, or a `./` path to a part of your own), its params, and an optional `not_simulated` reason.
- `wires` lists pin pairs, such as `["nano.D2", "btn1.a"]`, and `orbe` works out the nets. Every wire has a kind. `jumper` is the default and is drawn. `trace` (copper on a board) and `contact` (two pins touching, such as a leg in a breadboard hole) are not drawn. Drawing traces comes later.
- `layout` sits in the parent, keyed by name, with `{ "at": [x, y, z], "rotate": [rx, ry, rz] }` in mm and degrees. The rotation is about the parent's fixed X, then Y, then Z, URDF's order. In a scene, `on` plugs a part into a board's holes in place of `at`, as [Breadboards](#breadboards-nov-8-2026) says. `mate` is reserved for later.
- A net takes the name of its smallest label. A scene can name nets in `nets`, such as `"GND": "nano.GND"`.
- Nested names flatten for the engine by joining with `__`, so `@nano.D13` is the net `nano__D13`. Names may not contain `__`.
- A viewer edit goes to the file that holds what it changes. If that file is used more than once, the viewer asks first. Sealed parts refuse edits. A part with no position is placed by the viewer for drawing only, and nothing is written until a person drags it.
- Two hashes: one over every project file read, which guards writes, and one over the netlist, which says whether the simulation is current.
- The ESP32-S3 takes one file param per flash image, `partitions` and `app`. Its ROM is a file in the chip part's folder, named by a file param's `default`.

## Checking (Nov 4 2026)
Type checking, linting and runtime diagnostics are separate.
- `orbe check` type-checks: do the files describe a valid circuit? Every finding is an error. `orbe open` runs it first.
- `orbe lint` runs `orbe check`, then lint rules that never block, such as an unnamed ground net, a part with no position or an unconnected pin. It includes the engine's static checks, such as a floating net.
- A part lists the pins its design leaves open in `open`, such as `"open": ["AREF"]` on the Nano and the Uno. `net.floating` is left out for the net of one of those pins while no scene wire reaches it and every other pin on the net, other than a hole, belongs to that part or a part inside it. A hole that only seats that pin, with nothing else in its strip, does not reach the net. A scene wire, another part's pin, or a wire to a hole on the net still warns. `open` does not change the model, so the part format stays 1. A binary that does not know the key refuses the part file.
- A part that holds both a source and a chip, such as a board, runs with nothing wired to it, so lint does not call it unconnected. The rule reads the engine model kinds, not part names.
- Runtime diagnostics come only from `run`, `step` and `test`.
- `orbe open` shows type errors and the engine's static warnings.

## The nano board (Nov 4 2026)
- It is built from 14 leaf parts, 28 children in all, wired with traces. The viewer draws it by composing its children's 3D models, with no merged model.
- Its header children sit at z = 0 with their plastic under the board, so it seats on a breadboard.
- A6 and A7 are not simulated yet, since the engine's 328P has the DIP's pins. A wire to them is an error that says so.
- Its 16 MHz clock is a three-terminal ceramic resonator, until a clone's photo says otherwise.

## The uno board (Nov 8 2026)
- It is drawn as the common CH340G clone, with the ATmega328P in a DIP-28 socket. Its files transcribe connectivity, values by role and measured positions, and they copy no drawing, text or logo. The only licensed source is the CC0 photo of the clone. Arduino's board and schematic files are not used, so every file of the part is MIT OR Apache-2.0.
- Its bare board is its own `model.py`: the outline, the holes, and a printed pin legend with no names or logos. It holds 21 leaf parts, 50 children in all.
- It runs from USB through F1 and Q1, as the Nano does. 3V3 comes from its LP2985, on the engine's fit. VIN, the barrel jack and the 5 V regulator are not simulated, and a wire to VIN is an error that says so.
- The engine has no P-channel MOSFET, so Q1 is a resistor at its on resistance, always on. R15 and R1, which bias its gate from VIN, are drawn but not simulated.
- D13 drives the L LED through 1k, as on RobotDyn's board. D13 with its pull-up on then reads low, about 1.45 V. A clone that buffers D13 through an LM358 reads high there.
- The 16 MHz clock is an HC-49 crystal, as the clone's photo shows. The crystal is not simulated. Its load capacitors are, held at 0 V as the resonator part holds its own, so each clock pin has a DC path.
- The capacitor on AREF starts where the circuit's DC solution puts it, with no held voltage. A 0 V hold would pull a rail wired to AREF to 0 V at t = 0. AREF with nothing wired to it has no DC path. The part lists `AREF` in `open`, and a hole that only seats the pin does not reach it, as [Checking](#checking-nov-4-2026) says.

## Breadboards (Nov 8 2026)
- Two parts: `breadboard-830`, BusBoard's BB830, with 63 columns and four 50-hole rails, and `breadboard-400`, a generic half-size board with 30 columns and four 25-hole rails. Each `model.py` builds its board from `packages/cad/cad/breadboard.py`.
- Every hole is a pin with one lead. A terminal hole is named by its row and column, `a1` to `j63`, as the board prints them. Rows a to e and f to j sit on either side of the 7.62 mm centre channel. The rails follow the BB830 datasheet's drawing (Rev 7, page 2). Beside row a, the outer rail `W` is + and the inner rail `X` is -. Beside row j, the inner rail `Y` is + and the outer rail `Z` is -. A rail's red or blue line runs on the side away from its partner rail. A rail hole takes the number of the column it sits beside, so `X20` is beside `a20`. Rail holes come in groups of five with a gap every sixth column, so some numbers have no rail hole, such as `W6`.
- The model lies with row a at -Y and column 1 at -X. That is the datasheet's drawing turned half a turn, not mirrored: the drawing has row A at the top and column 1 at the right. So the board's printed numbers run the way the drawing's do, read from the row j side.
- The datasheet gives no distance from a rail to the rows. The model puts the inner rail 3 pitches from row a or j and the outer rail 4, and both stay marked `# verify`. The Blocks rely on the inner rail's 7.62 mm: their 10.16 mm pull-down resistor, from row b, lands in `X`.
- `orbe part` and the hint for an unknown hole name only holes that exist. The holes read as rows that share their columns, as `rows W to Z at columns 1 to 5, 7 to 11`, and a strip reads as its runs of holes, so a rail lists its groups of five.
- Which holes are joined is data, in the part's `holes` field: `{ "depth": 8, "strips": [["a1", "b1", "c1", "d1", "e1"], …] }`. A strip is the five holes of a column on one side of the channel, or a whole rail. The reader joins a strip's holes as it joins a part's own traces. Two other ways were weighed. A child part per strip, each with its own model, would cost a node and a draw call per strip. Part `wires` of kind `trace` would take 700 pairs on the 830, which the viewer must skip. A clip in a breadboard is not copper on a board either, and the viewer, hover and lint need each strip whole, which a list of pairs leaves them to rebuild.
- The 830's rails run its whole length. Its datasheet gives four 50-hole power rails and draws each as one. A part for a board with split rails lists each half as its own strip.
- A hole's anchor is a socket cut into the body, so it has `"node": null` and the merge keeps no node for it. A breadboard is four meshes, one per material, so it costs 4 draw calls however many holes it has. The asset step checks that each such socket has a vertex of the model within 1 mm of its mouth.
- A hole names no net, and a net that reaches no engine pin is never written. So empty holes add no unknowns to solve and no lint warnings, and a net through a strip takes the name it would take wired pin to pin. Each Block's netlist is the same bytes on a breadboard as wired directly, and a test keeps it so. The same netlist gives the same run folder.
- A part plugs in through `layout`, with `"r1": { "on": { "a": "bb.d25" } }`. The named pin's leg goes into the named hole. `rotate` turns the part about Z on the board, and the part turns with the board. Every other leg must land in a hole too. The part then shifts by the mean of its legs' offsets, and each leg must sit within 1 mm of its hole's centre, as a lead flexes, so a 6 mm switch's 6.5 by 4.5 mm legs fit 7.62 by 5.08 mm of holes. The part's seating plane sits on the board's top. A leg longer than the hole is deep raises the part, as an LED stands on its legs. Each leg's pin then joins its hole as a `contact` wire does.
- `on` needs a board with `holes` and an `at`, and the board turns about Z only. A part takes `on` or `at`, not both, and nothing plugs into a part that is itself plugged in.
- A pin on two or more legs of one part goes in by its lowest lead: a switch's `a` by lead 1, an `atmega328p-dip28`'s `GND` by lead 8. A scene names pins, not leads, so `on` has no other way to choose. Naming the hole another of the pin's legs would land in seats the part elsewhere, a different circuit that checks, so the skill shows that case. A label on two leaf pins, such as the Nano's `GND`, names no one hole, so `on` refuses it.
- `orbe check` fails a leg in the centre channel or off the board, a leg more than 1 mm from its hole, two legs in one hole, a wire end in a hole a leg or another wire end already names, and a hole or pin the part lacks. A leg error names the leg and the hole. A shared hole names the hole and both leads. When the strip has a free hole the hint is `a hole holds one lead; move one of them to a free hole in the same strip, such as bb.c1`. When every hole in the strip is taken the hint is `a hole holds one lead, and every hole in this strip is taken; free one first, for example by moving one of its leads to an empty strip and wiring that strip back to the hole it left`. When a free hole remains but each one is already named by another hint in this check, the hint is `a hole holds one lead; every free hole in this strip is already named by another hint in this check`. Two pins of one part in one strip stays the `scene.shorted` warning.
- A missed leg's hint is a placement that was tried. It is the first quarter turn of 0, 90, 180 and -90 from the same hole that seats every leg in a free hole with no strip joining two of the part's pins. Else it is the nearest hole within 10.16 mm, with a quarter turn, that does. Else the hint says no quarter turn seats the part there. Quarter turns are on the board's grid, so a resistor at 45 degrees is offered 0, not 135.
- Jumpers stay `wires`, as `["bb.c1", "bb.c25"]`, and are drawn from hole to hole. A jumper may still end on a pin off the board, such as the Uno's headers.
- `orbe lint` warns `scene.shorted` when two pins of one part sit in one strip, such as a resistor along a column, and its hint is found the same way.
- Hover on a hole gives its ref and its strip, as `@bb.b20` and `strip bb.a20 to bb.e20`.
- `on` was weighed against two other forms. With `at` and a `contact` wire per leg, the agent works out millimetres from the hole grid and lists every leg, the scene repeats the board's geometry, and a leg one hole off gives a different circuit with no error. `mate` would join parts by named features in general, which is more than a hole needs, and it stays reserved. `on` takes one hole, and every other leg follows from the part's own anchors, so an agent writes a hole name and gets an error that names the leg that missed.

## Later
Mates between parts, joints between two parts, importing outside CAD, PCB routing, drawing traces, per-instance placement overrides, A6 and A7, and an `orbe lsp` for editors.

## Repo (Oct 31 2026)
One private repo for the whole branch: `orbefab/embedded`. The current sim-engine code moves in first, then maker-sandbox, then the rest (Blocks and skills). The sandbox then builds against the current engine instead of a frozen copy.

## Parts model (Oct 31 2026)
Every part (circuit parts, sensors, dev boards) should eventually have its sim model, specs and 3D model, organized together. An inventory of what exists is the next step.

## 3D models (Oct 31 2026)
`orbefab/models` is where 3D models are built. The finished models a part needs end up living in this repo with that part.

## Part model scripts (Nov 8 2026)
A part's 3D model is `parts/<id>/model.py`. The script uses build123d and the helpers in `packages/cad`. `scripts/model.sh` builds `<id>.glb` and runs the asset step. Rebuild only the parts you changed. The same bytes on every run are proven on Linux x86_64. A part with no `model.py` yet still comes from the commit its `asset.json` records.

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
