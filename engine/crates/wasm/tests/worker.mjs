// The wasm package in node worker threads, against native `sim run` byte for byte.
//
// Usage: node crates/wasm/tests/worker.mjs PKG_DIR EXAMPLES_DIR NATIVE_DIR SIM WORK_DIR
//
// NATIVE_DIR/<name> holds `sim run`'s folder for EXAMPLES_DIR/<name>/netlist.json. Each example
// runs in its own Worker, as do the two input cases, which write their export and its native
// folder under WORK_DIR, and the argument checks. `scripts/wasm-check.sh` sets this up.

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads';

// One 60 Hz frame of a run paced at 1x.
const FRAME_PS = 16_666_666_667n;

// The input cases: the part each takes inputs on, and which setting a seeded input writes.
const INPUTS = [
  { name: 'atmega-button', part: 'sw1', setting: 'contact' },
  { name: 'atmega-analog-serial', part: 'rv1', setting: 'level' },
  { name: 'v-serial', part: 'v1', setting: 'serial' },
];

// `bigint` arguments the binding takes exactly, and the `created_at` each stamps as seconds.
const EDGES = [
  [0n, '1970-01-01T00:00:00Z'],
  [2n ** 53n + 1n, '285428751-11-12T07:36:33Z'],
  [2n ** 63n, '292277026596-12-04T15:30:08Z'],
  [2n ** 64n - 1n, '584554051223-11-09T07:00:15Z'],
];

// `bigint` arguments outside 0 to 2^64 - 1, which the binding refuses with `range`.
const OUTSIDE = [-1n, 2n ** 64n, 2n ** 64n + 1n, 2n ** 64n + 5n, 2n ** 128n];

// Values of another JavaScript type for each argument type, which the binding refuses with
// `type`. Each would otherwise be coerced, reduced, read as other bytes, or trap the module.
const WRONG = {
  Uint8Array: [new ArrayBuffer(2), '{}', [123, 125], {}, 5, null, undefined],
  bigint: [5, '5', true, null, undefined],
  string: [5, 5n, true, {}, null, undefined],
  boolean: ['true', 'false', 1, 0, 1n, {}, [], null, undefined],
  number: ['0.3', 1n, true, {}, null, undefined],
};

// Strings that are not a name in the ref grammar, which the binding refuses with `bad_name`.
const BAD_NAMES = ['1rv', '', 'rv1 ', 'r'.repeat(300), 'rv\0'];

// A seeded xorshift, so a failing chunking replays.
class Rng {
  constructor(seed) {
    this.state = BigInt.asUintN(64, seed);
  }

  below(n) {
    let x = this.state;
    x = BigInt.asUintN(64, x ^ (x << 13n));
    x ^= x >> 7n;
    x = BigInt.asUintN(64, x ^ (x << 17n));
    this.state = x;
    return x % n;
  }
}

// Every file under `dir`, by its path with `/` separators, in the order `Folder.paths()` lists.
function nativeFolder(dir, prefix = '') {
  const files = new Map();
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = prefix + entry.name;
    if (entry.isDirectory()) {
      for (const [inner, bytes] of nativeFolder(join(dir, entry.name), `${path}/`)) {
        files.set(inner, bytes);
      }
    } else {
      files.set(path, readFileSync(join(dir, entry.name)));
    }
  }
  return new Map([...files].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
}

// `folder`, stamped 0 seconds, holds the files of the native folder in `dir`, byte for byte.
// `sim run` stamps the wall clock, so the native `manifest.json` is compared with its
// `created_at` value replaced by the epoch.
function assertSameFolder(label, folder, dir) {
  const want = nativeFolder(dir);
  assert.deepEqual(folder.paths(), [...want.keys()], `${label}: the folder's paths`);
  for (const [path, bytes] of want) {
    const got = Buffer.from(folder.file(path));
    let native = bytes;
    if (path === 'manifest.json') {
      const stamp = /"created_at": "[^"]*"/g;
      assert.equal(bytes.toString().match(stamp)?.length, 1, `${label}: one created_at`);
      native = Buffer.from(bytes.toString().replace(stamp, '"created_at": "1970-01-01T00:00:00Z"'));
    }
    assert.ok(got.equals(native), `${label}: ${path} is the native bytes`);
  }
}

// The rows of `serial.csv` in the native folder in `dir`, and none without one.
function serialRows(dir) {
  try {
    return readFileSync(join(dir, 'traces', 'serial.csv'), 'utf8').split('\n').slice(1, -1);
  } catch {
    return [];
  }
}

// The probes read at row times are those rows of the native trace in `dir`, column for column.
function assertProbesOnRows(label, { refs, probes }, dir) {
  const index = JSON.parse(readFileSync(join(dir, 'traces', 'index.json')));
  const time = readFileSync(join(dir, 'traces', 'time.u64'));
  const analog = readFileSync(join(dir, 'traces', 'analog.f64'));
  const columns = index.analog.columns.map((column) => column.ref);
  assert.deepEqual(refs, columns, `${label}: refs() is the trace columns in order`);
  const times = Array.from({ length: index.rows }, (_, r) => time.readBigUInt64LE(8 * r));
  for (const { at, values } of probes) {
    const r = times.indexOf(at);
    assert.ok(r >= 0, `${label}: ${at} ps is a row time`);
    const row = columns.map((_, k) => analog.readDoubleLE(8 * (k * index.rows + r)));
    assert.deepEqual(values, row, `${label}: the probe at ${at} ps is that row`);
  }
}

// Advances `session` in seeded calls until it ends. A call moves 1 ps, a frame, to the next row
// time, or up to an eighth of the run. A frame is 60 Hz at 1x, or a 60th of a shorter run.
// After each call it reads the probe, the levels and the new serial bytes, and keeps the probes
// at row times. Then `input` may apply an input at `now`. Returns the refs, the probes kept, and
// the serial bytes as `serial.csv` rows.
function advanceInChunks(session, json, rng, input = () => {}) {
  const { stop_ps, grid_ps } = JSON.parse(json).analysis.transient;
  const [stop, grid] = [BigInt(stop_ps), BigInt(grid_ps)];
  const isRow = (at) => at === stop || at % grid === 0n;
  const frame = FRAME_PS < stop / 60n ? FRAME_PS : stop / 60n + 1n;
  const refs = session.refs();
  const signals = session.signals();
  const usarts = JSON.parse(session.usarts());
  const probes = [];
  const rows = [];
  let cursor = 0n;
  for (;;) {
    const now = session.now();
    let stride;
    switch (Number(rng.below(8n))) {
      case 0:
      case 1:
        stride = 1n;
        break;
      case 2:
        stride = (now / grid + 1n) * grid - now;
        break;
      case 3:
        stride = 1n + rng.below(stop / 8n + 1n);
        break;
      default:
        stride = frame;
    }
    const status = session.advanceTo(now + stride);
    const at = session.now();
    const probe = session.probe();
    assert.equal(probe.at, at, 'the probe is at now');
    if (isRow(at)) {
      probes.push({ at, values: Array.from(probe.values) });
    }
    probe.free();
    assert.equal(session.levels().length, signals.length, 'a level for each signal');
    const serial = session.serialSince(cursor);
    serial.byte.forEach((byte, k) => {
      const [part, usart] = usarts[serial.usart[k]];
      rows.push(`${serial.at[k]},${part},${usart},${byte}`);
    });
    cursor = serial.cursor;
    serial.free();
    if (status !== 'running') {
      return { refs, probes, rows };
    }
    input(session, rng);
  }
}

// `run` is the native folder. A transient recorded, advanced in chunks and finished is that
// folder too, and its serial rows are those of `serial.csv`. `record` refuses an `op` analysis.
function example(wasm, job, json, native) {
  assertSameFolder(`${job.name} run`, wasm.run(json, 0n), native);
  if ('op' in JSON.parse(json).analysis) {
    assert.throws(
      () => wasm.Session.record(json),
      (error) => error instanceof wasm.SimError && error.code === 'op',
      'record refuses an op analysis with op',
    );
    return 'run is sim run, record refuses op';
  }
  const session = wasm.Session.record(json);
  const read = advanceInChunks(session, json, new Rng(job.seed));
  assert.equal(session.status(), 'stopped');
  assertSameFolder(`${job.name} finish`, session.finish(0n), native);
  assertProbesOnRows(job.name, read, native);
  assert.deepEqual(read.rows, serialRows(native), `${job.name}: the serial rows are serial.csv's`);
  const counts = `${read.probes.length} probes on rows, ${read.rows.length} serial rows`;
  return `run and finish are sim run, ${counts}`;
}

// The setting `part` holds after its last event in `session.export()`, and that event's time.
// A contact is `true` for closed. A level is the position. A serial send is its baud, frame
// and bytes. None of those is the document's initial setting when the part has no event yet.
function lastSetting(session, { part, setting }) {
  const found = JSON.parse(Buffer.from(session.export())).parts.find((item) => item.name === part);
  if (setting === 'contact') {
    const last = found.events.at(-1);
    return { at: last && BigInt(last.t_ps), value: (last?.state ?? found.initial) === 'closed' };
  }
  if (setting === 'serial') {
    const last = found.voltage.events.at(-1);
    return {
      at: last && BigInt(last.t_ps),
      value: { baud: last.baud, frame: last.frame, bytes: last.bytes },
    };
  }
  const { kind: shape, initial, events, value } = found.position;
  const last = shape === 'steps' ? events.at(-1) : undefined;
  return { at: last && BigInt(last.t_ps), value: last?.level.value ?? initial?.value ?? value };
}

// Rates other than 9600. A binding that stores 9600 for every accepted rate fails the export.
const SERIAL_BAUDS = [115200, 500_000_000, 1, 333_333_333];

// One 8N1 byte at a drawn baud on `part` at `now`. It lands in the export at `Applied.at`.
// Whether it landed.
function applySeededSerial(wasm, session, rng, input) {
  const { part } = input;
  const bytes = [Number(rng.below(256n))];
  const baud = SERIAL_BAUDS[Number(rng.below(BigInt(SERIAL_BAUDS.length)))];
  const before = lastSetting(session, input);
  let applied;
  try {
    applied = session.applySerial(part, session.now(), baud, new Uint8Array(bytes));
  } catch (error) {
    assert.ok(error instanceof wasm.SimError, `${part} throws a SimError`);
    assert.equal(error.code, 'past_stop', `${part} lands or is past the stop`);
    assert.deepEqual(lastSetting(session, input), before, `${part} past the stop is unchanged`);
    return false;
  }
  const at = applied.at;
  applied.free();
  assert.deepEqual(
    lastSetting(session, input),
    { at, value: { baud, frame: '8N1', bytes } },
    `${part} holds the byte from Applied.at`,
  );
  return true;
}

// One seeded input on `part` at `now`, a contact, a position from 0.1 to 0.9, or one serial
// byte. A setting equal to the part's last throws `no_change`, and then a contact is moved to
// the other one. An input that lands is in the export at `Applied.at` with the setting passed.
// Whether it landed.
function applySeeded(wasm, session, rng, input) {
  if (input.setting === 'serial') {
    return applySeededSerial(wasm, session, rng, input);
  }
  const { part } = input;
  const contact = input.setting === 'contact';
  const apply = (setting) =>
    contact
      ? session.applyContact(part, session.now(), setting)
      : session.applyLevel(part, session.now(), setting);
  const before = lastSetting(session, input);
  let setting = contact ? rng.below(2n) === 0n : Number(1n + rng.below(9n)) / 10;
  if (setting === before.value) {
    assert.throws(
      () => apply(setting),
      (error) => error instanceof wasm.SimError && error.code === 'no_change',
      `${part} at ${setting}, its last setting, throws no_change`,
    );
    if (!contact) {
      return false;
    }
    setting = !setting;
  }
  let applied;
  try {
    applied = apply(setting);
  } catch (error) {
    assert.ok(error instanceof wasm.SimError, `${part} at ${setting} throws a SimError`);
    assert.equal(error.code, 'past_stop', `${part} at ${setting} lands or is past the stop`);
    assert.deepEqual(lastSetting(session, input), before, `${part} past the stop is unchanged`);
    return false;
  }
  const at = applied.at;
  applied.free();
  assert.deepEqual(lastSetting(session, input), { at, value: setting }, `${part} holds ${setting} from Applied.at`);
  return true;
}

// A recorded session with seeded inputs at chunk ends, advanced to the document stop. Its
// ended export, run by native `sim`, is the folder `finish` writes, and the serial rows read are
// that folder's `serial.csv` rows.
function withInputs(wasm, job, json, work) {
  const label = `${job.name} with inputs`;
  const session = wasm.Session.record(json);
  let applied = 0;
  const read = advanceInChunks(session, json, new Rng(job.seed), (session, rng) => {
    if ((applied === 0 || rng.below(2n) === 1n) && applySeeded(wasm, session, rng, job)) {
      applied += 1;
    }
  });
  assert.ok(applied > 0, `${label}: an input landed`);
  assert.equal(session.status(), 'stopped');
  const exported = join(work, `${job.name}.json`);
  const native = join(work, job.name);
  mkdirSync(work, { recursive: true });
  writeFileSync(exported, session.export());
  execFileSync(workerData.sim, ['run', exported, '--out', native]);
  assertSameFolder(label, session.finish(0n), native);
  assertProbesOnRows(label, read, native);
  assert.deepEqual(read.rows, serialRows(native), `${label}: the serial rows are serial.csv's`);
  const counts = `${read.probes.length} probes on rows, ${read.rows.length} serial rows`;
  return `finish is sim run on the ended export after ${applied} inputs, ${counts}`;
}

// Each argument of another type in WRONG throws `type`, a string that is not a name throws
// `bad_name`, and each `bigint` in OUTSIDE throws `range`, all moving nothing. A call with two
// wrong arguments throws the first one's code. A detached netlist throws `type`, and 1100 of them,
// past the module's 1024 externref slots, leave it working. Each of EDGES crosses exactly, as a
// time, a cursor, an input time and seconds.
function argumentChecks(wasm, examples) {
  const read = (name) => readFileSync(join(examples, name, 'netlist.json'));
  const isCode = (code) => (error) => error instanceof wasm.SimError && error.code === code;
  const shown = (value) => {
    if (typeof value === 'bigint') return `${value}n`;
    if (value instanceof ArrayBuffer) return 'an ArrayBuffer';
    if (value instanceof Uint8Array) return 'a detached Uint8Array';
    return value === undefined ? 'undefined' : JSON.stringify(value).slice(0, 12);
  };
  const steps = read('v-steps');
  const serial = wasm.Session.open(read('atmega-analog-serial'));
  const folder = wasm.run(steps, 0n);
  const calls = [
    ['Session.open', 'Uint8Array', (v) => wasm.Session.open(v)],
    ['Session.record', 'Uint8Array', (v) => wasm.Session.record(v)],
    ['run netlist', 'Uint8Array', (v) => wasm.run(v, 0n)],
    ['run createdAtSeconds', 'bigint', (v) => wasm.run(steps, v)],
    ['advanceTo', 'bigint', (v) => serial.advanceTo(v)],
    ['serialSince', 'bigint', (v) => serial.serialSince(v)],
    ['applyLevel part', 'string', (v) => serial.applyLevel(v, 0n, 0.5)],
    ['applyLevel ps', 'bigint', (v) => serial.applyLevel('rv1', v, 0.5)],
    ['applyLevel si', 'number', (v) => serial.applyLevel('rv1', 0n, v)],
    ['applyContact part', 'string', (v) => serial.applyContact(v, 0n, true)],
    ['applyContact ps', 'bigint', (v) => serial.applyContact('rv1', v, true)],
    ['applyContact closed', 'boolean', (v) => serial.applyContact('rv1', 0n, v)],
    ['applySerial part', 'string', (v) => serial.applySerial(v, 0n, 9600, new Uint8Array([1]))],
    ['applySerial ps', 'bigint', (v) => serial.applySerial('rv1', v, 9600, new Uint8Array([1]))],
    ['applySerial baud', 'number', (v) => serial.applySerial('rv1', 0n, v, new Uint8Array([1]))],
    ['applySerial bytes', 'Uint8Array', (v) => serial.applySerial('rv1', 0n, 9600, v)],
    ['finish', 'bigint', (v) => wasm.Session.record(steps).finish(v)],
    ['Folder.file', 'string', (v) => folder.file(v)],
  ];
  let refused = 0;
  const refuses = (what, call, values, code) => {
    for (const value of values) {
      assert.throws(() => call(value), isCode(code), `${what}(${shown(value)}) throws ${code}`);
      refused += 1;
    }
  };
  for (const [what, type, call] of calls) {
    refuses(what, call, WRONG[type], 'type');
    if (type === 'bigint') {
      refuses(what, call, OUTSIDE, 'range');
    }
  }
  refuses('applyLevel part', (part) => serial.applyLevel(part, 0n, 0.5), BAD_NAMES, 'bad_name');
  refuses('applyContact part', (part) => serial.applyContact(part, 0n, true), BAD_NAMES, 'bad_name');
  refuses(
    'applySerial part',
    (part) => serial.applySerial(part, 0n, 9600, new Uint8Array([1])),
    BAD_NAMES,
    'bad_name',
  );
  refuses('applyLevel si', (si) => serial.applyLevel('rv1', 0n, si), [NaN, Infinity], 'not_finite');
  refuses(
    'applySerial baud',
    (rate) => serial.applySerial('rv1', 0n, rate, new Uint8Array([1])),
    [0, 0.5, 1.5, -1, 500_000_001, NaN, Infinity],
    'bad_baud',
  );
  const firstWrong = [
    ['applyContact("1rv", -1n, "x")', () => serial.applyContact('1rv', -1n, 'x'), 'bad_name'],
    ['applyContact("rv1", -1n, "x")', () => serial.applyContact('rv1', -1n, 'x'), 'range'],
    ['applyLevel("1rv", -1n, "x")', () => serial.applyLevel('1rv', -1n, 'x'), 'bad_name'],
    ['applyLevel("rv1", -1n, NaN)', () => serial.applyLevel('rv1', -1n, NaN), 'range'],
    [
      'applySerial("1rv", -1n, 9600, bytes)',
      () => serial.applySerial('1rv', -1n, 9600, new Uint8Array([1])),
      'bad_name',
    ],
    [
      'applySerial("rv1", -1n, "x", bytes)',
      () => serial.applySerial('rv1', -1n, 'x', new Uint8Array([1])),
      'range',
    ],
    [
      'applySerial("rv1", 0n, 0, "x")',
      () => serial.applySerial('rv1', 0n, 0, 'x'),
      'bad_baud',
    ],
    ['run("x", -1n)', () => wasm.run('x', -1n), 'type'],
  ];
  for (const [what, call, code] of firstWrong) {
    assert.throws(call, isCode(code), `${what} throws ${code}, the first wrong argument's`);
    refused += 1;
  }
  const detached = () => {
    const bytes = new Uint8Array(steps);
    structuredClone(bytes.buffer, { transfer: [bytes.buffer] });
    return bytes;
  };
  for (const [what, , call] of calls.slice(0, 3)) {
    refuses(what, call, [detached()], 'type');
  }
  refuses(
    'applySerial bytes',
    (bytes) => serial.applySerial('rv1', 0n, 9600, bytes),
    [detached()],
    'type',
  );
  for (let k = 0; k < 1100; k += 1) {
    assert.throws(() => wasm.run(detached(), 0n), isCode('type'), `detached netlist ${k} throws type`);
  }
  assert.throws(() => serial.applyContact('rv1', 0n, false), isCode('wrong_setting'), 'a boolean is a contact');
  const stepsSession = wasm.Session.open(steps);
  assert.throws(
    () => stepsSession.applySerial('v1', 0n, 9600, new Uint8Array([1])),
    isCode('wrong_setting'),
    'bytes on a steps source throw wrong_setting',
  );
  const serialSource = wasm.Session.open(read('v-serial'));
  assert.throws(
    () => serialSource.applySerial('v1', 0n, 9600, new Uint8Array()),
    isCode('no_change'),
    'an empty send throws no_change',
  );
  assert.equal(serial.now(), 0n, 'a refused argument moves nothing');
  assert.equal(JSON.parse(Buffer.from(serial.export())).parts[1].position.value, 0.49951171875);
  assert.equal(folder.file('nothing'), undefined, 'a missing file is undefined');
  const padded = Buffer.concat([Buffer.from('junk'), steps, Buffer.from('junk')]);
  const view = new Uint8Array(padded.buffer, padded.byteOffset + 4, steps.length);
  assert.ok(Buffer.from(wasm.run(view, 0n).file('netlist.json')).equals(steps), 'a view is its own bytes');

  for (const [ps] of EDGES.slice(1)) {
    assert.throws(() => serial.applyLevel('rv1', ps, 0.5), isCode('past_stop'), `${ps}n is past the stop`);
  }
  assert.equal(serial.advanceTo(2n ** 64n - 1n), 'stopped');
  const sent = BigInt(serial.serialSince(0n).byte.length);
  assert.ok(sent > 0n, 'the session sent bytes');
  for (const [cursor] of EDGES) {
    const next = serial.serialSince(cursor).cursor;
    assert.equal(next, cursor === 0n ? sent : cursor, `serialSince(${cursor}n) keeps the cursor`);
  }

  const stop = 2n ** 64n - 1n;
  const far = Buffer.from(
    steps
      .toString()
      .replace(
        /"stop_ps": \d+, "max_step_ps": \d+, "grid_ps": \d+/,
        `"stop_ps": ${stop}, "max_step_ps": ${stop}, "grid_ps": ${stop}`,
      ),
  );
  const session = wasm.Session.open(far);
  for (const [ps] of EDGES) {
    session.advanceTo(ps);
    assert.equal(session.now(), ps, `advanceTo(${ps}n) stands at ${ps}n`);
  }
  assert.equal(session.status(), 'stopped');
  for (const [seconds, stamp] of EDGES) {
    const folders = { run: wasm.run(steps, seconds), finish: wasm.Session.record(steps).finish(seconds) };
    for (const [call, folder] of Object.entries(folders)) {
      const { created_at } = JSON.parse(Buffer.from(folder.file('manifest.json')));
      assert.equal(created_at, stamp, `${call}(${seconds}n) stamps ${stamp}`);
    }
  }
  return `${refused} refused arguments across ${calls.length} arguments, 1100 detached netlists, ${EDGES.length} edges cross exactly`;
}

if (isMainThread) {
  const [pkg, examples, native, sim, work] = process.argv.slice(2);
  if (!work) {
    console.error('usage: worker.mjs PKG_DIR EXAMPLES_DIR NATIVE_DIR SIM WORK_DIR');
    process.exit(2);
  }
  const names = readdirSync(examples, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && existsSync(join(examples, entry.name, 'netlist.json')))
    .map((entry) => entry.name)
    .sort();
  assert.ok(names.length >= 14, 'every example is here');
  const jobs = [
    ...names.map((name, k) => ({ kind: 'example', name, seed: 0x5eed_c000n + BigInt(k) })),
    ...INPUTS.map((input, k) => ({ ...input, kind: 'inputs', seed: 0x5eed_c100n + BigInt(k) })),
    { kind: 'arguments', name: 'arguments' },
  ];
  const results = await Promise.all(
    jobs.map(
      (job) =>
        new Promise((resolve) => {
          const worker = new Worker(new URL(import.meta.url), {
            workerData: { job, pkg, examples, native, sim, work },
          });
          worker.on('message', (checked) => resolve({ job, checked }));
          worker.on('error', (error) => resolve({ job, error }));
          worker.on('exit', (code) => resolve({ job, error: `exited ${code} with no result` }));
        }),
    ),
  );
  let failed = 0;
  let inputPosts = 0;
  for (const { job, checked, error } of results) {
    const inputRow = INPUTS.find((row) => row.name === job.name && row.part === job.part);
    if (inputRow) {
      inputPosts += 1;
      const text = typeof checked === 'string' ? checked : '';
      const posted =
        job.kind === 'inputs' &&
        !error &&
        text.startsWith('finish is sim run on the ended export');
      if (!posted) {
        failed += 1;
        console.log(
          `FAIL ${inputRow.name} with inputs: kind ${job.kind}, ${error?.stack ?? error ?? checked}`,
        );
        continue;
      }
    }
    const label = job.kind === 'inputs' ? `${job.name} with inputs` : job.name;
    if (error) {
      failed += 1;
      console.log(`FAIL ${label}: ${error.stack ?? error}`);
    } else {
      console.log(`ok   ${label}: ${checked}`);
    }
  }
  if (inputPosts !== INPUTS.length) {
    failed += 1;
    console.log(`FAIL inputs: ${inputPosts} results, want ${INPUTS.length}`);
  }
  console.log(`${results.length - failed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
} else {
  const { job, pkg, examples, native, work } = workerData;
  const wasm = await import(pathToFileURL(join(pkg, 'sim_wasm.js')));
  wasm.initSync({ module: readFileSync(join(pkg, 'sim_wasm_bg.wasm')) });
  const json = () => readFileSync(join(examples, job.name, 'netlist.json'));
  const checks = {
    example: () => example(wasm, job, json(), join(native, job.name)),
    inputs: () => withInputs(wasm, job, json(), work),
    arguments: () => argumentChecks(wasm, examples),
  };
  parentPort.postMessage(checks[job.kind]());
}
