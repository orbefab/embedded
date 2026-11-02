# Decisions

Investigations may challenge any of these, with reasons.

## Firmware toolchains are not ours (Oct 31 2026)
The engine runs firmware built with any toolchain, and users bring whatever they like. Orbefab never builds its own compiler. The skills recommend existing tools: arduino-cli for AVR/Arduino, ESP-IDF for ESP32.

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
- A board is a group of components shipped together as one library part, and it can be broken down into its sub-components. Same for other assemblies.
- Prior art: sim-all-proto ADR 0010 (parts nest) and the layered-sim Uno/Nano composites. Today's sim-engine board example is flat.
- The exact nesting format goes to the parts and scene-format investigations.

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
- The viewer sends input commands only. It can't restart a session. Its edits go to the scene file and show as not simulated until the next open.

## No 2D views (Nov 2 2026)
Parts have no 2D schematic view. The top-down view is a camera on the 3D scene.

## Engine naming (Nov 1 2026)
Inside this repo the simulator is just "engine" (perhaps; may change if it clashes with a visualization engine). `orbefab/sim-engine` isn't renamed; it's archived once its code has moved.
