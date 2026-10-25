# WebAssembly

`crates/wasm`, package `sim-wasm`, puts the session and `sim run` in a JavaScript package. A page runs a circuit live in a Worker, reads it every frame, applies inputs, and gets the same run folder `sim run` writes, byte for byte. [engine.md](engine.md#sessions) says what a session does. This page says how it crosses into JavaScript.

## Build

`scripts/wasm.sh [OUT_DIR]` builds the package into `OUT_DIR`, by default `.scratch/wasm/pkg` beside the repo. It writes `sim_wasm.js`, `sim_wasm_bg.wasm`, `sim_wasm.d.ts` and `sim_wasm_bg.wasm.d.ts`. It adds the `wasm32-unknown-unknown` target when rustup lacks it, and exits 2 when rustup is missing.

The `wasm-bindgen` CLI must be the version `Cargo.lock` holds for the `wasm-bindgen` crate, 0.2.129 today. The script uses `WASM_TOOLS_DIR/wasm-bindgen-<version>` when it is there, then a `wasm-bindgen` on `PATH` at that version. Otherwise it installs that version there from source, which takes about a minute once. `WASM_TOOLS_DIR` defaults to `.scratch/wasm/tools` beside the repo, so worktrees that set one directory share one install. Two installs at once each build in their own directory, and the first to finish is kept. It ignores a `PATH` copy at another version and says so. When the install fails, offline for example, it exits 1 and prints the command to run. `Cargo.toml` pins the crate with `=`, so a `cargo update` cannot move it away from the CLI. Moving it means changing the pin.

The package is `wasm-bindgen --target web` output. It has no `package.json`, and nothing publishes it.

## Load it in a Worker

One package loads in a browser module Worker and in a node `worker_threads` Worker. In a browser, the default export fetches `sim_wasm_bg.wasm` beside `sim_wasm.js`.

```js
import init, { Session } from './sim_wasm.js';
await init();
```

In node, pass the module's bytes. Node reads `sim_wasm.js` as a module from 22.7 on. An older node reads it as CommonJS and fails, unless a `package.json` with `"type": "module"` sits beside it.

```js
import { readFileSync } from 'node:fs';
import { initSync, Session } from './sim_wasm.js';
initSync({ module: readFileSync(new URL('./sim_wasm_bg.wasm', import.meta.url)) });
```

A session advanced in frames, reading what a frame draws. `vrx` is a `serial` source the netlist declares. Apply at `session.now()` after an advance that has passed the firmware's `RXEN0` store, and only while `status` is `running`. A send before that store is lost, as on hardware. `ps` is that session time. The binding reads no clock, so a wall-clock reading is not a session time. `applySerial` does not run the byte it sends. `serialSince` in the same turn does not yet hold it. The call on a later frame reads the echo, once a step has landed at or after the end of its stop bit. That later advance has to stay inside `stop_ps`. A netlist shorter than one 60 Hz frame ends in the first advance, so the loop below steps by 1 ms. The binding copies `bytes` into an owned array before `applySerial` returns, so the caller can reuse the `Uint8Array`.

```js
const session = Session.record(netlistBytes);
const frame = 1_000_000_000n; // 1 ms, so a later advance stays inside a short stop
let cursor = 0n;
let status = 'running';
let sent = false;
for (let t = frame; status === 'running'; t += frame) {
  status = session.advanceTo(t);
  if (!sent && status === 'running') {
    const applied = session.applySerial('vrx', session.now(), 9600, new Uint8Array([0x31]));
    applied.free();
    sent = true;
  }
  const probe = session.probe();
  draw(session.refs(), probe.values, session.levels());
  probe.free();
  const serial = session.serialSince(cursor);
  print(serial.byte);
  cursor = serial.cursor;
  serial.free();
}
const folder = session.finish(BigInt(Math.floor(Date.now() / 1000)));
```

## The surface

Times are `bigint` picoseconds both ways, so a run past 2^53 ps, about 2.5 hours, stays exact. `finish` and `run` take a `bigint` of unix seconds for `created_at`. Every `bigint` argument, a time, a cursor or seconds, must be from 0 to 2^64 - 1, and one outside that range throws `range`. The binding reads the argument as a JavaScript value and checks it, so it is never reduced modulo 2^64. The binding reads no clock. Per-frame numbers cross as typed arrays. Names and refs cross as strings in the ref grammar of [netlist.md](netlist.md). A diagnostic and the USART list cross as JSON text, in the schema the run folder writes, for the caller to parse. Every method name below is on `Session` unless it names its class.

| JavaScript | What it does | Crosses as |
| --- | --- | --- |
| `Session.open(netlist)` | `open_document`, a session that records nothing | `Uint8Array` in |
| `Session.record(netlist)` | `record_document`, a session `finish` turns into a folder | `Uint8Array` in |
| `advanceTo(ps)` | accepts every step by `ps` | `bigint` in, `Status` out |
| `now()`, `status()` | where the session stands | `bigint`, `Status` |
| `fault()` | the diagnostic that ended a faulted session | JSON text of one diagnostic, or `undefined` |
| `refs()` | the analog columns, the trace's, fixed for the session | `string[]` |
| `probe()` | every analog column at `now`, and each column's integral from t = 0 | a `Probe` with `at` (`bigint`), `values` and `integrals` (`Float64Array` in `refs()` order) and `interpolated` |
| `signals()`, `levels()` | each chip signal, and its level at `now` | `string[]`, and `Uint8Array` with 0 low and 1 high |
| `usarts()` | each USART as its part and name | JSON text such as `[["u1","USART0"]]` |
| `serialSince(cursor)` | the bytes after the first `cursor` | `bigint` in, a `Serial` with `at` (`BigUint64Array`), `usart` (`Uint16Array`, an index into `usarts()`), `byte` (`Uint8Array`) and `cursor` (`bigint`) |
| `applyContact(part, ps, closed)` | sets a switch's contact | `string`, `bigint`, `boolean` in, an `Applied` with `at` (`bigint`) and `status` |
| `applyLevel(part, ps, si)` | sets a `steps` source or a potentiometer, in volts, amperes or the position fraction | `string`, `bigint`, `number` in, an `Applied` |
| `applySerial(part, ps, baud, bytes)` | sends 8N1 frames on a `serial` source | `string`, `bigint`, `number`, `Uint8Array` in, an `Applied` |
| `export()` | the netlist with every applied input | `Uint8Array` |
| `finish(createdAtSeconds)` | advances to the stop and returns the folder `sim run` writes, and frees the session, also when it throws | `bigint` in, a `Folder` |
| `run(netlist, createdAtSeconds)` | the folder `sim run` writes, as one call | `Uint8Array` and `bigint` in, a `Folder` |
| `Folder.paths()`, `Folder.file(path)` | the folder's files, such as `traces/analog.f64` | `string[]`, and `string` in, `Uint8Array` or `undefined` out |

`Status` is `"running"`, `"stopped"` or `"faulted"`. The `.d.ts` files type every class, and `Status` and the error codes as string unions.

A probe's `integrals` run from t = 0 to `at`, in the column's unit times picoseconds. The mean on an interval is the difference of two readings divided by the width in picoseconds.

A cursor is the count of serial bytes read, so `0n` reads from the first byte, and two readers can each keep their own. A cursor past the end reads nothing and stays.

`finish` on a session with no input writes the bytes `record` read as `netlist.json`. After an input it writes the export of the ended session, whose `stop_ps` is the document's. `sim run` on that export writes the same folder, `created_at` aside. An export taken while the session runs has a shorter `stop_ps`, so it is not that netlist. To keep the export `finish` wrote, advance to the stop, call `export()`, then `finish`.

`run` is the batch path. A netlist that `record` refuses still gets its folder from `run`, as `sim run` writes one. That covers an `op` analysis, an invalid netlist and a grid past the row limit.

## Errors

Every failure the engine or the binding detects throws a `SimError`. It is not an `Error`, so test it with `instanceof SimError`. `code` says which failure it is. `diagnostics` is JSON text of an array, empty unless the code says otherwise. `rule` is set only for `value`.

Each argument must have the JavaScript type the table above names, and one of another type throws `type`. Nothing is converted, so `"0.3"` for a level, `"true"` or `1` for a contact, `5` for a time and `null` for any argument each throw `type`. A netlist must be a `Uint8Array`. A node `Buffer` is one, and a view of part of a buffer crosses as just its bytes. An `ArrayBuffer`, such as `response.arrayBuffer()` returns, or a string throws `type`, so wrap it in a `Uint8Array` or encode it first. A `Uint8Array` whose buffer was transferred, so it is detached, throws `type` too. The binding checks the arguments in order, so the code is that of the first wrong one. A call that throws changes nothing, except `finish`, which has freed its session.

One failure throws from the `wasm-bindgen` glue instead. A call on an object that `finish` or `free()` has freed throws an `Error` whose message is `null pointer passed to rust`.

| `code` | Thrown by | Means |
| --- | --- | --- |
| `op` | `open`, `record` | the analysis is `op`, which has no time to advance |
| `failed` | `open`, `record` | `diagnostics` holds the one error, such as `solver.too_large`, a failed operating point at t = 0, or for `record` `trace.too_large` |
| `invalid` | `open`, `record` | the bytes are not a valid netlist, and `diagnostics` holds why |
| `ended` | `applyContact`, `applyLevel`, `applySerial` | the session has stopped or faulted |
| `unknown_part` | the same | no part has that name |
| `not_settable` | the same | the part takes no input of that kind. A serial send on a resistor, a capacitor, an inductor, a relay, a junction, a chip, or a `step` or `pulse` source |
| `wrong_setting` | the same | a contact on a source or a potentiometer, a level on a switch, a serial send on a switch, a potentiometer, or a plain or `steps` source, or a contact or a level on a `serial` source |
| `value` | `applyLevel` | the level breaks a rule, and `rule` names it as `diagnostics.json` does, such as `fraction` for a position outside (0, 1) |
| `past_stop` | `applyContact`, `applyLevel`, `applySerial` | the input would land at or after `stop_ps`, or a serial send's frames would not end inside a time the engine can store |
| `no_change` | the same | the setting equals the part's last, or a serial send has no bytes |
| `type` | every call that takes an argument | an argument is not of the JavaScript type the table above names, or a `Uint8Array` argument is detached |
| `bad_name` | `applyContact`, `applyLevel`, `applySerial` | the part name is a string but not a name in the ref grammar |
| `not_finite` | `applyLevel` | the level is NaN or infinite |
| `bad_baud` | `applySerial` | `baud` is not an integer from 1 to 500000000 |
| `range` | `advanceTo`, `serialSince`, `applyContact`, `applyLevel`, `applySerial`, `finish`, `run` | a `bigint` argument is negative or above 2^64 - 1 |
| `unrecorded` | `finish` | `open` made the session, so it has no folder. `finish` has freed the session |

A `failed` from `record` lists only the error. The floating-net warnings `sim run` reports beside it are not there. `run` on the same bytes writes a folder whose `diagnostics.json` lists them.

## Freeing

Each object holds memory inside the module until it is freed. `free()` frees it at once. An object the page drops is freed when the garbage collector finalizes it, which can be much later, so a loop that reads every frame should free each `Probe` and `Serial`. `finish` frees its session, also when it throws, and any later call on it throws an `Error`. A thrown `SimError` is an object of its own too.

## The serial log

A session keeps every serial byte it delivers, 16 bytes of memory each, and no call trims the log. At 115200 baud without pause the log grows by about 663 MB an hour, and in a 4 GiB wasm32 memory it fails to grow after about 3.2 hours. [engine.md](engine.md#sessions) gives the sizing. A recorded session needs every byte for `serial.csv`. The first caller that keeps an unrecorded session open for over an hour at 115200 baud is the reason to add a trim, one that drops the bytes before a cursor and keeps cursors counting from the first byte.

## Tests

`crates/wasm/tests/adapter.rs` calls the binding's methods natively, in `scripts/check.sh`. It checks the statuses, each `SimError` code but `type` and `range`, the cursor, `unrecorded`, and the `created_at` a folder is stamped with. It opens `atmega-blink` as the binding and as the session under it, and checks that `now`, `status`, the probe with its `interpolated` flag, `signals` and `levels` are the session's through three LED toggles. `atmega-button` checks that a contact lands where the session lands it, with the session's status. `atmega-analog-serial` checks that each position is stored as the number passed, by comparing the export with the session's. On `v-steps` an input of 1e200 V faults the run, and `Applied.status` must say `faulted` as the session's does. On `v-serial`, a send at 115200 baud lands where the session lands it and the exports match, a baud that is not an integer from 1 to 500000000 throws `bad_baud`, and an empty send throws `no_change`. Bytes on `v-steps` throw `wrong_setting`. A copy of `atmega-analog-serial` with a second chip checks that each serial byte names its own USART. With Newton's row budget lowered to 1, through a tests-only feature of `sim-circuit`, `v-pulse-rc` checks a probe that interpolates. Natively each argument is its Rust type, so the checks of `type` and `range` run only in the Worker test. The part name grammar and the finite level check are one function on both targets, so the adapter test covers them for the package too.

`scripts/wasm-check.sh [SCRATCH_DIR]` builds the package, builds `sim`, and runs `sim run` on every example. Then `crates/wasm/tests/worker.mjs` runs each example in its own node Worker. `run` must write the native folder byte for byte. `sim run` stamps `manifest.json` with the wall clock, so the Worker replaces the one `created_at` value in the native bytes with the epoch's and compares the bytes. A transient recorded and advanced in seeded calls of 1 ps, 60 Hz frames, row times and long strides must `finish` to that folder. Each probe it read at a row time must be that row of `analog.f64`, and the serial bytes it read must be the rows of `serial.csv`. The integrals at the stop must match a second session opened on the same bytes and advanced to the stop in one call, bit for bit. `record` must refuse each `op` example with `op`. `atmega-button` takes seeded contacts, `atmega-analog-serial` seeded positions, and `v-serial` and `atmega-serial-echo` seeded bytes at call ends. After each input that lands, the part's last event in `export()` must be at `Applied.at` and hold the contact, the position, or the baud, frame and bytes passed. A setting equal to the part's last must throw `no_change`. An empty serial send must throw `no_change` too. A baud that is not an integer from 1 to 500000000 must throw `bad_baud`. Bytes on a `steps` source must throw `wrong_setting`. A detached `Uint8Array` passed to `applySerial` must throw `type`. Their `finish` must be native `sim run` on the ended export. One more Worker checks the arguments. Every argument of every call must throw `type` for values of other JavaScript types, such as a number, a string or `null` for a `bigint`, `"true"` or `1` for a boolean, and an `ArrayBuffer`, a string or an array for a netlist. Each `bigint` argument must throw `range` for -1, 2^64, 2^64 + 1, 2^64 + 5 and 2^128. A part name that is a string but not a name must throw `bad_name`, and none of these may move the session. A call with two wrong arguments must throw the first one's code. A detached netlist must throw `type`, and 1100 of them must leave the module working. 0, 2^53 + 1, 2^63 and 2^64 - 1 must cross exactly as a time, a cursor and seconds. Before the Worker runs, the script checks that `sim_wasm.d.ts` types each argument as the table above does, that `Probe` carries `integrals` as a `Float64Array`, and that its `Code` union is the codes of the Errors table in order. The script needs node 22.7 or later and rustup. It exits 2 and names the one that is missing, or the older node it found. It is not in `check.sh`, which must pass on a machine without them.
