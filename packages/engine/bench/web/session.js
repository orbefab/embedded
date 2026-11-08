import init, { Session } from "./pkg/orbe_engine_wasm.js";

// 1e12 / 60 rounded. docs/engine/wasm.md advances a 60 Hz frame by this many picoseconds.
const FRAME_PS = 16_666_666_667n;

function describe(err) {
  if (err && err.code) {
    const diagnostics = err.diagnostics ? ` ${err.diagnostics}` : "";
    return `${err.code}${diagnostics}`;
  }
  if (err && err.message) {
    return String(err.message);
  }
  return String(err);
}

function spread(values) {
  const sorted = [...values].sort((a, b) => a - b);
  return {
    median: sorted[Math.floor(sorted.length / 2)],
    min: sorted[0],
    max: sorted[sorted.length - 1],
  };
}

function stopPs(bytes) {
  const text = new TextDecoder().decode(bytes);
  const match = /"stop_ps"\s*:\s*(\d+)/.exec(text);
  if (!match) {
    throw new Error("netlist has no stop_ps");
  }
  return BigInt(match[1]);
}

// The count uses this literal, not FRAME_PS, so a longer step still fails it.
function sandboxFrames(stop) {
  const frame = 16_666_666_667n;
  return (stop + frame - 1n) / frame;
}

function runOnce(bytes, stop, label, example) {
  const opened = performance.now();
  const session = Session.open(bytes);
  try {
    const refCount = session.refs().length;
    const signalCount = session.signals().length;
    if (refCount < 1 || signalCount < 1) {
      throw new Error(`${label} has ${refCount} refs and ${signalCount} signals`);
    }
    let status = "running";
    let cursor = 0n;
    let frames = 0;
    let probeReads = 0;
    let levelReads = 0;
    let serialReads = 0;
    let serialBytes = 0;
    let probeColumns = 0;
    let levelCount = 0;
    for (let t = FRAME_PS; status === "running"; t += FRAME_PS) {
      const at = t < stop ? t : stop;
      status = session.advanceTo(at);
      const names = session.refs();
      const probe = session.probe();
      const columns = probe.values.length;
      probe.free();
      if (columns !== names.length) {
        throw new Error(`${label} probe has ${columns} columns and ${names.length} refs`);
      }
      probeColumns += columns;
      probeReads += 1;
      const levels = session.levels();
      if (levels.length !== signalCount) {
        throw new Error(
          `${label} levels has ${levels.length} signals and the netlist has ${signalCount}`,
        );
      }
      levelCount += levels.length;
      levelReads += 1;
      const serial = session.serialSince(cursor);
      serialBytes += serial.byte.length;
      cursor = serial.cursor;
      serial.free();
      serialReads += 1;
      frames += 1;
      if (at === stop) {
        break;
      }
    }
    const now = session.now();
    const wallNs = Math.round((performance.now() - opened) * 1e6);
    const expected = sandboxFrames(stop);
    if (BigInt(frames) !== expected || status !== "stopped" || now !== stop) {
      const ran = frames === 1 ? "frame" : "frames";
      const want = expected === 1n ? "frame" : "frames";
      throw new Error(
        `${label} ended ${status} at ${now} ps across ${frames} ${ran} and stop is ${stop} ps in ${expected} ${want}`,
      );
    }
    if (probeReads !== frames || levelReads !== frames || serialReads !== frames) {
      throw new Error(
        `${label} read probe ${probeReads}, levels ${levelReads}, serial ${serialReads} across ${frames} frames`,
      );
    }
    if (BigInt(serialBytes) !== cursor) {
      throw new Error(`${label} read ${serialBytes} serial bytes and the cursor is ${cursor}`);
    }
    if (example === "atmega-analog-serial" && serialBytes < 1) {
      throw new Error(`${label} read no serial bytes`);
    }
    if (probeColumns !== frames * refCount || levelCount !== frames * signalCount) {
      throw new Error(
        `${label} probe has ${probeColumns} columns for ${refCount} refs and levels ${levelCount} for ${signalCount} signals across ${frames} frames`,
      );
    }
    return {
      wallNs,
      frames,
      probeReads,
      levelReads,
      serialReads,
      serialBytes,
      probeColumns,
      levelCount,
      status,
      now: now.toString(),
      cursor: cursor.toString(),
      framePs: 16666666667,
      refs: refCount,
      signals: signalCount,
    };
  } finally {
    session.free();
  }
}

function row(job, samples) {
  const first = samples[0];
  for (const sample of samples) {
    if (
      sample.frames !== first.frames ||
      sample.serialBytes !== first.serialBytes ||
      sample.probeColumns !== first.probeColumns ||
      sample.levelCount !== first.levelCount ||
      sample.now !== first.now ||
      sample.cursor !== first.cursor ||
      sample.refs !== first.refs ||
      sample.signals !== first.signals
    ) {
      throw new Error(`${job.example} ${job.variant} reads changed across runs`);
    }
  }
  return {
    example: job.example,
    variant: job.variant,
    status: first.status,
    now_ps: first.now,
    frames: first.frames,
    frame_ps: first.framePs,
    probe_reads: first.probeReads,
    level_reads: first.levelReads,
    serial_reads: first.serialReads,
    serial_bytes: first.serialBytes,
    cursor: first.cursor,
    probe_columns: first.probeColumns,
    level_count: first.levelCount,
    refs: first.refs,
    signals: first.signals,
    wall_ns: spread(samples.map((sample) => sample.wallNs)),
  };
}

self.onmessage = async (event) => {
  try {
    const runs = event.data.runs;
    if (!Number.isInteger(runs) || runs < 1) {
      throw new Error("runs must be at least 1");
    }
    await init();
    const jobs = await (await fetch("session-jobs.json")).json();
    const ready = [];
    for (const job of jobs) {
      const response = await fetch(job.file);
      if (!response.ok) {
        throw new Error(`missing ${job.example} ${job.variant}`);
      }
      const bytes = new Uint8Array(await response.arrayBuffer());
      ready.push({ job, bytes, stop: stopPs(bytes) });
    }
    if (ready.length > 0) {
      const warm = Session.open(ready[0].bytes);
      warm.free();
    }
    const workloads = [];
    for (const item of ready) {
      const label = `${item.job.example} ${item.job.variant}`;
      const samples = [];
      for (let i = 0; i < runs; i += 1) {
        samples.push(runOnce(item.bytes, item.stop, label, item.job.example));
      }
      workloads.push(row(item.job, samples));
    }
    self.postMessage({ report: { target: "worker-session", runs, workloads } });
  } catch (err) {
    self.postMessage({ error: describe(err) });
  }
};
