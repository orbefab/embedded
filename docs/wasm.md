# The browser package

`packages/wasm`, crate and package `orbe-wasm`, runs a Block live in a browser Web Worker. It is the scene reader and the session `orbe` runs, compiled to WebAssembly, and it speaks to the viewer in the messages `orbe show`'s WebSocket carries. A page hands it a Block's files, plays it, sends it the viewer's commands, and draws what comes back. [viewer.md](viewer.md#run-a-block-in-the-page) says how the viewer page does that.

The CLI and the Worker share one implementation of what a viewer sees. `orbe_session::viewer` reads a viewer's command, answers it, and builds the `scene` and `frame` messages on open, after each command and on each `play` tick. The session process calls it for its WebSocket, and `orbe-wasm` calls it for the Worker.

## Build

```sh
scripts/wasm.sh [OUT_DIR]
```

It builds the package into `OUT_DIR`, by default `.scratch/wasm/pkg` in the repo. The folder holds `orbe_wasm.js`, `orbe_wasm_bg.wasm`, `orbe_wasm.d.ts`, `orbe_wasm_bg.wasm.d.ts`, `worker.js`, `package.json`, `LICENSE-MIT` and `LICENSE-APACHE`. It needs bash 4.2 or later and rustup with the `wasm32-unknown-unknown` target, which `rust-toolchain.toml` lists.

It also needs the `wasm-bindgen` CLI at the version `Cargo.lock` holds for the `wasm-bindgen` crate, 0.2.129 today. `Cargo.toml` pins the crate with `=`, so a `cargo update` cannot move it away from the CLI. The script uses `WASM_TOOLS_DIR/wasm-bindgen-<version>/bin/wasm-bindgen` when it is there, then a `wasm-bindgen` on `PATH` at that version, and ignores a `PATH` copy at another version with a line that says so. `WASM_TOOLS_DIR` defaults to `.scratch/wasm/tools` in the repo. The script installs nothing. Without the CLI it exits 2 and prints the command that installs it where the script looks:

```sh
cargo install wasm-bindgen-cli --version =0.2.129 --locked --root .scratch/wasm/tools/wasm-bindgen-0.2.129
```

Without the target it exits 2 and prints `rustup target add wasm32-unknown-unknown`.

A wasm32 build of `orbe-scene` embeds no catalog and no Blocks, so the package holds none of `parts/` or `blocks/`. A page hands it the parts a Block runs, from a registry.

### Reproducible

Two builds of one commit write the same bytes, whatever the target directory. The `cargo build` passes `--remap-path-prefix` on that invocation only, so a native build does not inherit the flags. The prefixes are the checkout, `CARGO_HOME`, the sysroot printed by `$RUSTC` or `rustc`, and the target directory when it sits outside the checkout. `CARGO_HOME`, the sysroot and the target directory are each passed as the path that was set and as the physical path. The replacements are `/orbe`, `/cargo`, `/rustc` and `/target`. The shortest prefix comes first, because rustc keeps the last prefix that matches.

The build takes no extra rustflags. The script puts only the remap flags in `CARGO_ENCODED_RUSTFLAGS` for that cargo process, and unsets `RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS` and `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS` on it. It exits 1 when `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS` or `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS` is set to a non-empty value, or when `$CARGO_HOME/config`, `$CARGO_HOME/config.toml`, or a `.cargo/config` or `.cargo/config.toml` from the build directory up to the root contains the text `rustflags`. The message names the variable or the file.

### `package.json`

`package.json` names the package `orbe-wasm`. The version is `[workspace.package] version` in the root `Cargo.toml`. `private` is true, so nothing can publish the folder by accident, and `license` is `MIT OR Apache-2.0`. `type` is `module`. `main`, `module` and `exports["."]` point at `orbe_wasm.js` and its types. `exports` also maps `./orbe_wasm_bg.wasm` and `./worker.js`. `files` lists the eight files. `orbe.commit` is `git rev-parse HEAD` when the directory is that checkout, with `-dirty` when `git status --porcelain --untracked-files=normal` lists a change. `ORBE_COMMIT`, when set, is the stamp instead, and must be 40 lowercase hex digits with an optional `-dirty`. Outside a checkout with no `ORBE_COMMIT`, `orbe.commit` is null and the script prints one warning. A consumer that vendors the package should refuse a null commit.

## Load it in a Worker

The package carries its own Worker, `worker.js`, which runs in a browser as a module Worker and in node as a `worker_threads` Worker. A page serves the package folder beside itself and starts it by URL:

```js
const worker = new Worker(new URL("orbe-wasm/worker.js", document.baseURI), { type: "module" });
worker.postMessage({ type: "open", name: "nano-button-led", files: [...files] });
worker.postMessage(JSON.stringify({ id: 1, cmd: "play" }));
worker.onmessage = (event) => draw(JSON.parse(event.data));
```

- The first message is `{ type: "open", name, files }`. `files` is an array of `[path, Uint8Array]` in the layout below, and `name` names the session in each `scene` message and reply.
- Every later message is one command as the JSON text a viewer sends on the WebSocket, such as `{"id": 1, "cmd": "press", "ref": "@btn1"}`.
- The Worker posts the JSON text of each message the WebSocket sends: one `scene` after the Block opens, a `frame` whenever the circuit shows something new, a `reply` to each command, and `closed` with the reason when the Block does not open. [viewer.md](viewer.md#the-messages) gives each message.
- While `play` runs, the Worker ticks every 16 ms of wall time, each tick by the wall time since the last, up to 16 ms, as the session process does. `performance.now` in the Worker is the only clock. The Rust code reads none.

`packages/viewer`'s `workerSource` speaks this protocol for the viewer, so a page rarely posts these messages itself.

### The files

A path under `parts/<id>/` is a file of the catalog part `id`, the folder a registry record for that part lists. Every other path is the Block's own, from its folder: `block.jsonc`, `scene.jsonc`, the firmware it names, and the rest. The session opens `scene.jsonc` as `orbe open` does in that folder. A Block reads every part from the `parts/` it is handed, and from nothing else, so a part missing there is `part_missing`.

### Without the Worker

`orbe_wasm.js` is `wasm-bindgen --target web` output. Its default export loads `orbe_wasm_bg.wasm` beside it in a browser. In node, pass the bytes:

```js
import { readFileSync } from "node:fs";
import { initSync, Block, Session } from "./orbe_wasm.js";
initSync({ module: readFileSync(new URL("./orbe_wasm_bg.wasm", import.meta.url)) });
```

A bundler that copies `orbe_wasm.js` into one bundle must also emit `orbe_wasm_bg.wasm` and pass its URL to `init({ module_or_path })`, or `init` looks for the file beside the bundle.

## The surface

| JavaScript | What it does |
| --- | --- |
| `new Block()` | An empty set of files. |
| `block.add(path, bytes)` | Adds one file, `string` and `Uint8Array`, in the layout above. |
| `Session.open(name, block)` | Opens the Block's `scene.jsonc` in a session named `name`. Its `scene` message and first `frame` wait in `messages()`. It takes the `Block`, which is freed. |
| `session.command(text)` | Runs one viewer command, given as its JSON text. The `reply`, then a `frame` when the circuit changed, wait in `messages()`. |
| `session.tick(wallMs)` | One `play` tick, `wallMs` milliseconds of wall time after the last. A playing session advances by that much, up to 16 ms, and the tick's `frame` carries `speed` and `ratio`. A paused session does not move. |
| `session.playing()` | Whether `play` runs, so the caller keeps ticking. |
| `session.messages()` | The JSON text of each message since the last call, in order. |
| `tickMs()` | The tick, 16 ms. |
| `partScene(id, block)` | The JSON text of the `scene` message of the catalog part `id` drawn on its own, with no session: one instance named `id` with each `-` written `_`, no `circuit_hash` and no channels. `block` holds the part's files and those of every part it is built from, at `parts/<id>/<path>`. It reads the part as far as its children, params and layout, and checks no param, firmware or ground, so a board with no firmware draws. It throws the findings when a part is missing or does not parse. A part's page draws the part with it. |

A viewer sends only `press`, `release`, `play` and `pause`, each parsed by the CLI's own command table. Each object holds memory inside the module until `free()` or the garbage collector frees it.

## Errors

`Session.open` throws an `Error` whose message is the refusal as `orbe open` prints it, such as `scene.jsonc:9:23: error[part_missing]: part `led-5mm` is not in the catalog this Block reads`, with its hint. `worker.js` posts that message as the `reason` of a `closed` message. A command the session refuses is not thrown: its `reply` carries `ok: false` and the error, as `orbe <command> --json` prints it. A command no viewer sends, or text that is not JSON, gets a `usage` error. `partScene` throws an `Error` whose message lists each finding as `orbe check` prints it. A call on a freed object throws `null pointer passed to rust`.

## Tests

`packages/wasm/tests/session.rs` runs the session natively in `scripts/check.sh fast`. It opens nano-button-led from its files and every catalog part, checks the `scene` then the first `frame`, plays in 16 ms ticks, presses `@btn1` until D13 is high and `@led1` carries over 5 mA, pauses, and checks that a paused session does not move and that `step` is refused. A Block without `led-5mm` is refused with the text `orbe open` prints. It also draws nano-board on its own with its 28 children and no channel, and refuses it with `part_missing` when `led-0603` is gone. `scripts/check.sh fast` also runs clippy on `orbe-wasm` for `wasm32-unknown-unknown`.

`scripts/check-wasm.sh [SCRATCH_DIR]` runs in `scripts/check.sh full`. It needs what `scripts/wasm.sh` needs and node 24 or later. It installs nothing.

- It builds the package, then builds it again with a fresh target directory, and fails unless both folders hold the same bytes.
- It checks `package.json`: the name, the version, `private`, the licence, `files`, each `exports` target, and a commit stamp when the directory is the checkout.
- It checks that `orbe_wasm.d.ts` types `Block.add`, `Session.open`, `command`, `tick`, `playing`, `messages` and `tickMs` as the table above does.
- It rejects the package when a file holds a host path. A file fails when it contains `$HOME/`, unless `HOME` is `/` or one of `/orbe`, `/cargo`, `/rustc` and `/target`, or a remapped host prefix with a trailing `/`. A match is a leak only when it starts a path: walking back from it over `/` bytes reaches the start of the file or a byte outside `[A-Za-z0-9._/-]`. A host path glued on after other path characters, such as `abc/home/u/x`, is a known miss. Before the build, a fixture that only names replacements must pass under `HOME` of each replacement, and a fixture that holds `/probe-home/secret` must fail under `HOME` of `/probe-home`.
- It builds `orbe`, builds this repo's registry with `orbe registry build`, and runs `packages/wasm/tests/worker.mjs` on the package and that registry.

`worker.mjs` loads each Block with the viewer's registry client, `packages/viewer/src/registry.ts`, which checks every size and hash as [registry.md](registry.md#verify-what-you-fetch) says, and opens it in the package's `worker.js` in a node worker thread, as a page does. nano-button-led and uno-button-led must open with D13, `@led1` and `@btn1` named, play to 100 ms with D13 low and `@led1` dark, light `@led1` past 5 mA with D13 high once `@btn1` is pressed, go dark once it is released, answer `pause`, and refuse `step` as `usage`. The Uno's `@uno.led_l` must light with `@led1`. nano-led-fade must show D9 both high and low while it plays to 1.5 s, and `@led1`'s mean current over 1.2 s to 1.5 s must be over 1 mA and over twice its mean over 20 ms to 300 ms. nano-button-led without `led-5mm` must post `closed` with the `part_missing` refusal. Last, it opens nano-button-led in a worker thread of its own, plays it, ticks 16 ms at a time as fast as the thread runs, and prints the wall time of one simulated second, three times.

The page's headed check, `scripts/check-block.sh`, runs a Block from a registry in Chromium. [viewer.md](viewer.md#run-a-block-in-the-page) describes it.
