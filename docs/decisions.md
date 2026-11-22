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

## A flag before the command runs it (Nov 22 2026)
`orbe --json open` runs `open`, the same as `orbe open --json`. A flag may come before the command name. A repeated flag that the command takes once counts once. `--help` and `--version`, and their short forms, still come first.

## Parts (Nov 4 2026)
- A part is one real physical object, defined in one file, `part.jsonc`. Every field is optional except `format`. A part may have an engine model, children (`parts`, `wires`, `layout`), both or neither.
- A part's `format` covers its `model`. A netlist change that an existing part file cannot survive raises the part `format`.
- A part with no model that only joins the wires that reach its pins, such as a pin header, says so with `"joins_only": true`. `orbe lint` then asks it for no model.
- `"listed": false` keeps a part out of `orbe search` and the site's parts list. Left out or `true`, the part is offered. It does not change the model, so the part format stays 1.
- A scene is not a part. We ship parts only.
- A param is a number, a choice or a file. A choice picks a named set of values, such as an LED's colour, and may tint the 3D model. A param never changes pins or shape. An LED's colour is required.
- A part with children passes a param to a child with `{ "param": "<name>" }` on the child, as a prop passes through a component. The whole value passes, with no arithmetic for now.
- Catalog parts name the real object in `real` (maker, part number, datasheet), and every model value cites its source.
- A part's files, such as the ESP32-S3 ROM, live in its folder. A file param's `default` names one, as a path relative to the part file.
- A file param reads its file `"as": "text"`, into the model as a string, or `"as": "blob"`, as bytes. A blob param writes `{ "sha256", "bytes" }` at its `sets` pointer, and the reader hands the bytes to the engine beside the netlist. `"as": "blob"` needs part format 2, which is format 1 plus that, so only a part that uses a blob declares format 2. Scenes stay format 1.
- A part states its readings, such as an LED's `lit` above 2 mA and `dim` above 100 uA. `brightness` is the average forward current over the last 20 ms, the time since t = 0 when the session is younger. 2 mA is the lowest current of the forward fit. 100 uA is a hundredth of the 10 mA at which the datasheet rates luminous intensity, and ten times the 10 uA reverse-current maximum.
- An addressable LED's colour is a reading, `"of": "rgb"`: the red, green and blue each of its LEDs last latched, and when the part last latched. It is a value, not a light state, so `get` prints it as `rgb(255,0,0)` and `expect` checks it as `@led1 rgb(255,0,0)`. A word that starts with `#` is a comment on a command line, so a colour is not written in hex. A `view.glow` by that reading glows in the latched colour. A chain's `DOUT` and that reading need part format 3, which is format 2 plus them, so an older orbe says it needs a newer one, and a part before format 3 keeps its `DOUT` off the chain.
- `joints` say what moves, such as a button's plunger, and `controls.press` names the joint it drives. Joints between two parts come later.
- Versions are three numbers. A patch or minor version can't change a passing test.
- Parts that share an engine model, such as two packages of one chip, each hold a copy of its values. A check keeps the copies equal.

## Catalog and projects (Nov 4 2026)
- The catalog is a registry: a static index plus one archive per part version, shaped like shadcn's registry. `orbe` reads it itself.
- `orbe add <id>…` downloads the latest parts, and the parts they use, into the project's `parts/<id>/`. A part a board is built from is nested at `parts/<board>/parts/<dep>/`. `orbe add` installs the highest version. `id@version` is a usage error, and the hint is the bare id. A folder already at `parts/<id>/` for an id the command names is `part_exists`: the command writes nothing, and the hint says to remove it and run the same `orbe add`.
- `orbe add`, `orbe search`, `orbe part` and `orbe blocks` fetch. `orbe check`, `orbe open`, `orbe test` and `orbe show` never do. A missing folder is `part_missing`, and the hint is the `orbe add` command that downloads it.
- The copy is the project's. Editing it is the user's, and `orbe check`, `orbe open` and `orbe test` read `parts/<id>/` as it is. They do not compare it to the registry. The hash of a part's published files is the folder hash a registry record stores. Build output in the folder is not one of those files, and neither are `model.py` and `asset.json`. Those build the 3D model. The GLB is published. A fresh clone holds every part it downloaded and can check them offline.
- Parts from other registries are named `@<registry>/<id>`. The first release reads the official registry only.
- 3D models live in plain git, in the part's folder.

## Blocks (Nov 8 2026)
- A Block is a small circuit that works: a board, parts, firmware and tests. `blocks/<id>/` holds `block.jsonc`, `scene.jsonc`, `firmware/`, `build/<name>.ino.hex`, `tests/` and `README.md`. It replaces `examples/`.
- `block.jsonc` holds what a Block's catalog card needs from the Block itself: `block` (its id, the folder's name), `version`, `title`, `summary`, `board`, `tags` and `license`. It does not list parts, since the scene names them.
- A Block commits no part copies. A folder that holds `block.jsonc` is a Block, and the reader resolves each id its scene names from the `parts/` beside `blocks/`. A Block in this repo then always runs the parts of the commit it sits in, so a part change that breaks a Block fails that commit's checks.
- A Block is self-contained. Its sketch and the licences that image carries sit in `firmware/`. The scene names the image as `build/<name>.ino.hex`, the file `arduino-cli compile --output-dir build` writes, so a build overwrites the shipped image. The path works in the Block and after it is installed.
- If a project ignores `build/` in git, a fresh clone fails `orbe test` with `firmware_missing` until it builds, which is the honest result.
- `orbe add <block>` installs a Block into a project. It copies the parts the scene names into `parts/`, as `orbe add <part>` does, then writes `scene.jsonc`, `firmware/`, `build/<name>.ino.hex`, `tests/` and `README.md`. A part folder already there is `part_exists` and the command writes nothing. `block.jsonc` stays behind, so the installed project is not itself a Block.
- `orbe add` never merges a Block into a scene. A project that already holds a different `scene.jsonc`, or any other file the Block brings, is refused before anything is written, and the hint installs into an empty folder. A file already there with the Block's bytes is kept when the part folders are not there yet. When those folders are already there, the command is `part_exists` and names the folders to remove, then the same `orbe add`.
- Parts and Blocks share `orbe add`, so a Block id is never a catalog part id. A test fails when one is both, and one word names one of them.
- A Block has a version, like a part. `orbe add <block>` installs the latest version. Naming a version is refused, and the hint is the bare id.
- A Block's registry record lists every part its scene runs, at the versions in the same build. The build does not run the Block's tests. In this repo `scripts/check.sh` runs them against the parts of the same commit.

## Registry (Nov 8 2026)
- `orbe registry build <src> <out>` builds a registry from any folder of `registry.jsonc`, `parts/` and `blocks/`, so another registry is built with the same `orbe` its users run. The code is the `orbe-registry` library, and the cards come from `orbe-scene`, the same as `orbe part` prints.
- `registry.jsonc` names the registry, such as `@orbefab`, and its homepage, and the index carries both, so an index says which registry it is wherever it is mirrored or merged.
- Item types are `part` and `block`, with shadcn's field names.
- Every record sits at `<id>@<version>` and never changes once served, and its files sit at `files/<sha256>`. Hash dedupe is the only optimisation. A record has no `usedBy`, since that grows with the registry.
- The site serves the versions in this repo. A version that has left the repo is not republished.
- `released.lock` records the SHA-256 of the served record of each part and Block at the version a release served. It is empty until the first tag. A line stays after its folder is removed. `scripts/check.sh fast` fails a record that differs from its line at that version, and `scripts/released.sh` rewrites the file at a tag, keeping those lines and never moving a version backwards.
- The index and every record carry `"format": 1`. Clients ignore fields they do not know, and removing or changing a field raises the format, so the schemas allow fields they do not name.
- A field with no value is left out, never `null`.
- A dependency is `[@<registry>/]<id>@<version>`, though the build writes only bare ones today, so a parser written now reads cross-registry dependencies later.
- The URL template has one `{name}`, in its last path segment. A client fills it only with `index` and takes every other URL from the index and the records.
- Records stay camel case, as shadcn's are, and `orbe part --json` stays snake case. [registry.md](registry.md) gives the exact mapping and a test converts every catalog part both ways.
- A project holds one folder per part id it adds, `parts/<id>/`, and nests the parts that part is built from inside it. `orbe add` downloads the latest version from `@orbefab` at `https://orbefab.com/embedded/r/{name}.json`. [registry.md](registry.md) is the client contract.

## Scenes (Nov 4 2026)
- A scene is its own kind of file, usually `scene.jsonc`. Its keys are `format`, `$schema`, `parts`, `wires`, `layout`, `ground`, `nets`, `sim` and `world`. It has no `pins` or `params`, and no file can use a scene as a part.
- Scenes and part files are JSONC: JSON plus `//` comments and trailing commas, nothing more. A file may carry a `$schema` key, which `orbe` ignores. No schema file is generated.
- An instance has a `type` (a catalog id, or a `./` path to a part of your own), its params, and an optional `not_simulated` reason.
- `wires` lists pin pairs, such as `["nano.D2", "btn1.a"]`, and `orbe` works out the nets. Every wire has a kind. `jumper` is the default and is drawn. `trace` (copper on a board) and `contact` (two pins touching, such as a leg in a breadboard hole) are not drawn. Drawing traces comes later.
- `layout` sits in the parent, keyed by name, with `{ "at": [x, y, z], "rotate": [rx, ry, rz] }` in mm and degrees. The rotation is about the parent's fixed X, then Y, then Z, URDF's order. In a scene, `on` plugs a part into a board's holes in place of `at`, as [Breadboards](#breadboards-nov-8-2026) says. `mate` is reserved for later.
- A net takes the name of its smallest label. A hole never names a net. The ground net is named `GND` when the scene's `nets` does not give that name to another net. Otherwise it keeps the name it already has, such as `nano__GND` when `nets` uses `GND` for another net, or the name the scene gave this net. A scene can name nets in `nets`, such as `"GND": "nano.GND"`.
- Nested names flatten for the engine by joining with `__`, so `@nano.D13` is the net `nano__D13`. Names may not contain `__`.
- A viewer edit goes to the file that holds what it changes. If that file is used more than once, the viewer asks first. A part with no position is placed by the viewer for drawing only, and nothing is written until a person drags it.
- Two hashes: one over every project file read, which guards writes, and one over the netlist, which says whether the simulation is current.
- The ESP32-S3 takes one file param per flash image, `partitions` and `app`. Its ROM is a file in the chip part's folder, named by a file param's `default`.

## Checking (Nov 4 2026)
Type checking, linting and runtime diagnostics are separate.
- `orbe check` type-checks: do the files describe a valid circuit? Every finding is an error. `orbe open` runs it first.
- `orbe lint` runs `orbe check`, then lint rules that never block, such as a part with no position or an unconnected pin. It includes the engine's static checks, such as a floating net.
- A part lists the pins its design leaves open in `open`, such as `"open": ["AREF"]` on the Nano and the Uno. `net.floating` is left out for the net of one of those pins while no scene wire reaches it and every other pin on the net, other than a hole, belongs to that part or a part inside it. A hole that only seats that pin, with nothing else in its strip, does not reach the net. A scene wire, another part's pin, or a wire to a hole on the net still warns. `open` does not change the model, so the part format stays 1. A binary that does not know the key refuses the part file.
- A part that holds both a source and a chip, such as a board, runs with nothing wired to it, so lint does not call it unconnected. The rule reads the engine model kinds, not part names.
- Runtime diagnostics come only from `run`, `step` and `test`.
- `orbe open` shows type errors and the engine's static warnings.

## The nano board (Nov 4 2026)
- It is built from 14 leaf parts, 28 children in all, wired with traces. The viewer draws it by composing its children's 3D models, with no merged model.
- Its header children sit at z = 0 with their plastic under the board, so it seats on a breadboard.
- A6 and A7 read their voltage. They are the 328P's `ADC6` and `ADC7`, which the engine's `atmega328p` kind has as pins of the TQFP.
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
- `orbe check` fails a leg in the centre channel or off the board, a leg more than 1 mm from its hole, two legs in one hole, a wire end in a hole a leg or another wire end already names, and a hole or pin the part lacks. A leg error names the leg and the hole. A shared hole names the hole and both leads. Its hint lists every free hole in that strip, in strip order, as `a hole holds one lead. free holes in this strip: bb.c1, bb.e1`, or says `a hole holds one lead, and this strip has no free hole`. Two hints in one check may list the same hole. Two pins of one part in one strip stays the `scene.shorted` warning.
- A missed leg's hint is a placement that was tried. It is the first quarter turn of 0, 90, 180 and -90 from the same hole that seats every leg in a free hole with no strip joining two of the part's pins. Else it is the nearest hole within 10.16 mm, with a quarter turn, that does. Else the hint says no quarter turn seats the part there. Quarter turns are on the board's grid, so a resistor at 45 degrees is offered 0, not 135.
- Jumpers stay `wires`, as `["bb.c1", "bb.c25"]`, and are drawn from hole to hole. A jumper may still end on a pin off the board, such as the Uno's headers.
- `orbe lint` warns `scene.shorted` when two pins of one part sit in one strip, such as a resistor along a column, and its hint is found the same way.
- Hover on a hole gives its ref and its strip, as `@bb.b20` and `strip bb.a20 to bb.e20`.
- `on` was weighed against two other forms. With `at` and a `contact` wire per leg, the agent works out millimetres from the hole grid and lists every leg, the scene repeats the board's geometry, and a leg one hole off gives a different circuit with no error. `mate` would join parts by named features in general, which is more than a hole needs, and it stays reserved. `on` takes one hole, and every other leg follows from the part's own anchors, so an agent writes a hole name and gets an error that names the leg that missed.

## The CLI (Nov 21 2026)
- `orbe nets [scene]` prints each net and the pins on it. A net with no DC path to ground is `(floating)`, except a board net lint leaves quiet, such as an open AREF. A pin on no net is `not connected`. `--json` gives the same lists.
- Human text prints a current where the reply builds it, in A, mA, µA or nA. The digits are the shortest decimal in that unit that parses back to the same value. A current under 1 nA stays in A, in scientific form. A rating's `for … ps` duration prints in ms or s. `--json` keeps the exact number and the same field names. Serial text and words the user wrote stay as they are. Voltages and power stay as the engine reports them.
- `orbe get light`, `orbe get temperature` and `orbe get humidity` read a world quantity the scene declares, and `orbe get @world.light` reads the same. The value is the one in force at the session's time, so a `get` just after `set` shows the previous value until a `step`. `--json` names `quantity`, `value` and `unit`.
- `orbe set` of a world quantity to a value already scheduled succeeds and says so. When that value has not landed, the reply names the time it lands. `press` and `release` of a switch already in that state still fail with `no_change`.
- `orbe search` with no words lists every part a scene can place, grouped by its first category, then the Blocks. A part that only exists inside a board is left out. With words, the search is unchanged.
- `orbe part` adds one line on how the part plugs into a breadboard, derived from its anchors and the hole pitch. A part whose legs cannot sit one to a hole says when two legs would share a strip and short. `--json` is the card, unchanged.
- `scripts/install.sh` gives its downloads a 15 second connect timeout, three tries, and fails a transfer that stays under 1 byte/s for 30 seconds.

## Later
Mates between parts, joints between two parts, importing outside CAD, PCB routing, drawing traces, per-instance placement overrides, and an `orbe lsp` for editors.

## Repo (Oct 31 2026)
One private repo for the whole branch: `orbefab/embedded`. The current sim-engine code moves in first, then maker-sandbox, then the rest (Blocks and skills). The sandbox then builds against the current engine instead of a frozen copy.

## Parts model (Oct 31 2026)
Every part (circuit parts, sensors, dev boards) should eventually have its sim model, specs and 3D model, organized together. An inventory of what exists is the next step.

## 3D models (Oct 31 2026)
The finished models a part needs end up living in this repo with that part.

## Part model scripts (Nov 8 2026)
A part's 3D model is `parts/<id>/model.py`. The script uses build123d and the helpers in `packages/cad`. `scripts/model.sh` builds `<id>.glb` and runs the asset step. Rebuild only the parts you changed. The same bytes on every run are proven on Linux x86_64.

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
- Third-party CAD keeps its own license (SO-ARM100 Apache-2.0, Adafruit MIT).
- The Waveshare Bus Servo Adapter model has no license and can't go public; it would need our own remodel.

## Parts catalog timing (Oct 31 2026)
The old trigger (a second view, a second shell or ~15 parts) is met, since both web and local visualizers are planned. The catalog is designed in the parts-organization investigation and filled as the engine moves in, starting with the nano board.

## Visualizer shells (Oct 31 2026, framework Nov 2 2026)
A web visualizer (the sandbox) and a local-computer visualizer share one viewer: three.js with React Three Fiber, reusing maker-sandbox's bench.
- Locally it's a browser tab that the session process serves, with the page built into the CLI. Nothing else installs.
- In the sandbox the same page runs the engine's session loop in a Web Worker, compiled to WebAssembly.
- The viewer's world is Y-up in mm. Parts are authored in their own frame, mm with +Z up, and one mapping converts between the two.
- Whole-scene screenshots use an open viewer, or else a Chrome, Chromium or Edge already on the machine.
- The viewer sends input commands only. It can't restart a session. Its edits go to the file that holds what they change (see "Scenes") and show as not simulated until the next open.

## No 2D views (Nov 2 2026)
Parts have no 2D schematic view. The top-down view is a camera on the 3D scene.

## Engine naming (Nov 1 2026)
Inside this repo the simulator is just "engine" (perhaps; may change if it clashes with a visualization engine). `orbefab/sim-engine` isn't renamed; it's archived once its code has moved.
