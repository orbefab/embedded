self.onmessage = async (event) => {
  try {
    const runs = event.data.runs;
    const names = await (await fetch("names.json")).json();
    const wasm = await (await fetch("sim_bench.wasm")).arrayBuffer();
    const { instance } = await WebAssembly.instantiate(wasm, {
      env: { now_ms: () => performance.now() },
    });
    const { bench_alloc, bench_begin, bench_add, bench_finish, bench_result, memory } =
      instance.exports;
    bench_begin(runs);
    const encoder = new TextEncoder();
    for (const name of names) {
      const response = await fetch(`examples/${name}/netlist.json`);
      if (!response.ok) {
        throw new Error(`missing ${name}`);
      }
      const json = new Uint8Array(await response.arrayBuffer());
      const nameBytes = encoder.encode(name);
      const bytes = new Uint8Array(nameBytes.length + json.length);
      bytes.set(nameBytes, 0);
      bytes.set(json, nameBytes.length);
      const ptr = bench_alloc(bytes.length);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
      if (bench_add(nameBytes.length) !== 1) {
        throw new Error(`rejected ${name}`);
      }
    }
    const length = bench_finish();
    const out = new Uint8Array(memory.buffer, bench_result(), length);
    const report = JSON.parse(new TextDecoder().decode(out));
    if (report.error) {
      throw new Error(report.error);
    }
    self.postMessage({ report });
  } catch (err) {
    self.postMessage({ error: String(err && err.message ? err.message : err) });
  }
};
