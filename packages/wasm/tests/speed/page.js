// Times one Block in the package's worker.js, in a dedicated Worker or on this page's main thread.
// `window.speed(job)` opens the Block, plays it, and measures the wall time between the first
// frame at or past `warmMs` of simulated time and the first at or past `warmMs + simMs`.
// In a Worker run it also measures the wall time from starting the Worker to the first picture:
// the first playing frame whose screen glass shows a lit pixel for a Block with a screen, or the
// first playing frame for a Block with no screen.
// On the main thread, worker.js reads `self` as the page's port to it, so `self` is replaced by
// one that hands its messages to this page.
const realNow = performance.now.bind(performance);
const realSetTimeout = setTimeout;
const realClearTimeout = clearTimeout;
const RUN_LIMIT_MS = 120_000;

const fromBase64 = (text) => Uint8Array.from(atob(text), (c) => c.charCodeAt(0));

/** Whether any screen glass in `screens` has an opaque pixel at full scale on any channel.
 * Dark glass and an empty backlit LCD stay below 255 on every channel; lit OLED, LCD and
 * RGB565 pixels hit 255. */
function screenDrawn(screens) {
  if (!Array.isArray(screens)) return false;
  for (const screen of screens) {
    if (screen === null) continue;
    const rgba = fromBase64(screen.rgba);
    for (let k = 0; k < rgba.length; k += 4) {
      if (rgba[k + 3] === 255 && (rgba[k] === 255 || rgba[k + 1] === 255 || rgba[k + 2] === 255)) {
        return true;
      }
    }
  }
  return false;
}

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
  let hasScreen = false;
  let firstPictureMs = null;
  let from = null;
  let rate = null;
  let ticks = 0;
  return new Promise((done, fail) => {
    const limit = realSetTimeout(() => {
      const waiting = rate === null ? `reach ${endPs} ps` : "show a first picture";
      fail(new Error(`${block} did not ${waiting} within ${RUN_LIMIT_MS} ms`));
    }, RUN_LIMIT_MS);
    const finish = (result) => {
      realClearTimeout(limit);
      playing = false;
      send(JSON.stringify({ id: 2, cmd: "pause" }));
      done(result);
    };
    const abort = (error) => {
      realClearTimeout(limit);
      playing = false;
      fail(error);
    };
    const startAt = realNow();
    const receive = (text) => {
      const message = JSON.parse(text);
      if (message.type === "closed") return abort(new Error(`${block} closed: ${message.reason}`));
      if (message.type === "scene") {
        hasScreen = Array.isArray(message.channels?.screens) && message.channels.screens.length > 0;
        return send(JSON.stringify({ id: 1, cmd: "play" }));
      }
      if (message.type === "reply" && message.id === 1) {
        if (!message.reply.ok) return abort(new Error(`${block} refused play: ${text}`));
        playing = true;
        return;
      }
      if (message.type !== "frame") return;
      if (where === "worker" && firstPictureMs === null && hasScreen && screenDrawn(message.screens)) {
        firstPictureMs = realNow() - startAt;
      }
      if (!playing) return;
      if (message.state !== "playing") return abort(new Error(`${block} is ${message.state} at ${message.t_ps} ps`));
      if (where === "worker" && firstPictureMs === null && !hasScreen) {
        firstPictureMs = realNow() - startAt;
      }
      if (rate === null) {
        if (from === null) {
          if (message.t_ps >= warmPs) {
            from = { at: realNow(), ps: message.t_ps };
            if (profile) console.profile("speed");
          }
        } else {
          ticks += 1;
          if (message.t_ps >= endPs) {
            const wallMs = realNow() - from.at;
            if (profile) console.profileEnd("speed");
            rate = { wallMs, simMs: (message.t_ps - from.ps) / 1e9, ticks };
          }
        }
      }
      if (rate !== null && (where !== "worker" || firstPictureMs !== null)) {
        finish({ ...rate, firstPictureMs });
      }
    };
    start(where, receive).then(
      (sender) => {
        send = sender;
        send({ type: "open", name: block, files });
      },
      (error) => abort(error),
    );
  });
};
window.speedReady = true;
