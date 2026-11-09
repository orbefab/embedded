// Times one Block in the package's worker.js, in a dedicated Worker or on this page's main thread.
// `window.speed(job)` opens the Block, plays it, and measures the wall time between the first
// frame at or past `warmMs` of simulated time and the first at or past `warmMs + simMs`.
// On the main thread, worker.js reads `self` as the page's port to it, so `self` is replaced by
// one that hands its messages to this page.
const realNow = performance.now.bind(performance);
const realSetTimeout = setTimeout;
const RUN_LIMIT_MS = 120_000;

const fromBase64 = (text) => Uint8Array.from(atob(text), (c) => c.charCodeAt(0));

async function blockFiles(block) {
  const response = await fetch(`block/${block}`);
  if (!response.ok) throw new Error(`block/${block} answered ${response.status}`);
  return (await response.json()).map(([path, bytes]) => [path, fromBase64(bytes)]);
}

/** Starts worker.js where `where` says, and returns the function that sends it a message. */
async function start(where, receive) {
  if (where === "worker") {
    const worker = new Worker("timed-worker.js", { type: "module" });
    worker.onmessage = (event) => receive(event.data);
    worker.onerror = (event) => receive(JSON.stringify({ type: "closed", reason: `worker error: ${event.message}` }));
    return (data) => worker.postMessage(data);
  }
  let deliver = null;
  window.self = {
    set onmessage(handler) {
      deliver = handler;
    },
    postMessage: receive,
  };
  await import("./clock.js");
  await import("/pkg/worker.js");
  return (data) => queueMicrotask(() => deliver({ data }));
}

window.speed = async ({ block, where, warmMs, simMs, profile }) => {
  const files = await blockFiles(block);
  const warmPs = warmMs * 1e9;
  const endPs = (warmMs + simMs) * 1e9;
  let send = null;
  let playing = false;
  let from = null;
  let ticks = 0;
  return new Promise((done, fail) => {
    const limit = realSetTimeout(() => fail(new Error(`${block} did not reach ${endPs} ps within ${RUN_LIMIT_MS} ms`)), RUN_LIMIT_MS);
    const finish = (result) => {
      clearTimeout(limit);
      playing = false;
      send(JSON.stringify({ id: 2, cmd: "pause" }));
      done(result);
    };
    const receive = (text) => {
      const message = JSON.parse(text);
      if (message.type === "closed") return fail(new Error(`${block} closed: ${message.reason}`));
      if (message.type === "scene") return send(JSON.stringify({ id: 1, cmd: "play" }));
      if (message.type === "reply" && message.id === 1) {
        if (!message.reply.ok) return fail(new Error(`${block} refused play: ${text}`));
        playing = true;
        return;
      }
      if (message.type !== "frame" || !playing) return;
      if (message.state !== "playing") return fail(new Error(`${block} is ${message.state} at ${message.t_ps} ps`));
      if (from === null) {
        if (message.t_ps < warmPs) return;
        from = { at: realNow(), ps: message.t_ps };
        if (profile) console.profile("speed");
        return;
      }
      ticks += 1;
      if (message.t_ps < endPs) return;
      const wallMs = realNow() - from.at;
      if (profile) console.profileEnd("speed");
      finish({ wallMs, simMs: (message.t_ps - from.ps) / 1e9, ticks });
    };
    start(where, receive).then(
      (sender) => {
        send = sender;
        send({ type: "open", name: block, files });
      },
      (error) => fail(error),
    );
  });
};
window.speedReady = true;
