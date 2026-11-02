# Decisions

Investigations may challenge any of these, with reasons.

## Firmware toolchains are not ours (Oct 31 2026)
The engine runs firmware built with any toolchain, and users bring whatever they like. Orbefab never builds its own compiler. The skills recommend existing tools: arduino-cli for AVR/Arduino, ESP-IDF for ESP32.

## Repo (Oct 31 2026)
One private repo for the whole branch: `orbefab/embedded`. The current sim-engine code moves in first, then maker-sandbox, then the rest (Blocks, skills, examples). The sandbox then builds against the current engine instead of a frozen copy.

## Parts model (Oct 31 2026)
Every part (circuit parts, sensors, dev boards) should eventually have its sim model, specs, 3D model and 2D view, organized together. An inventory of what exists is the next step.

## 3D models (Oct 31 2026)
`orbefab/models` is where 3D models are built. The finished models a part needs end up living in this repo with that part.

## Naming and nesting (Oct 31 2026)
- Parts use generic names (e.g. `uno-board`, `led-5mm`), not brand or product names.
- Everything is a part, and parts can nest.
- ICs (e.g. the 328P) are black boxes: the smallest unit, never broken down further.
- A board is a group of components shipped together as one library part, and it can be broken down into its sub-components. Same for other assemblies.
- Prior art: sim-all-proto ADR 0010 (parts nest) and the layered-sim Uno/Nano composites. Today's sim-engine board example is flat.
- The exact nesting format goes to the parts and scene-format investigations.

## Complete-part order (Oct 31 2026)
The first boards taken to complete (sim, specs, 3D, 2D):
1. The uno board.
2. A plain ESP32-S3 dev board (just the dev board, no knob hardware).
3. The knob (Guition) as its own separate part, which also happens to use an ESP32-S3.
4. Then others, such as the nano.

The ESP32-C3 and the original ESP32 come later (sim-engine models only the S3 today).

## Licensing (Oct 31 2026)
- Part files here (sim models, specs, 3D, 2D) use the same permissive license as the code: MIT/Apache, with the exact pick still open.
- Our own models in `orbefab/models` are Orbefab copyright, so they can be relicensed.
- Third-party CAD keeps its own license (SO-ARM100 Apache-2.0, Adafruit MIT).
- The Waveshare Bus Servo Adapter model has no license and can't go public; it would need our own remodel.

## Parts catalog timing (Oct 31 2026)
The old trigger (a second view, a second shell or ~15 parts) is met, since both web and local visualizers are planned. The catalog is designed in the parts-organization investigation and filled as the engine moves in, starting with the uno board.

## Visualizer shells (Oct 31 2026)
A web visualizer (the sandbox) and a local-computer visualizer, ideally sharing one underlying visualization engine. Framework and architecture are not picked; that's an investigation.

## Engine naming (Nov 1 2026)
Inside this repo the simulator is just "engine" (perhaps; may change if it clashes with a visualization engine). `orbefab/sim-engine` isn't renamed; it's archived once its code has moved.
