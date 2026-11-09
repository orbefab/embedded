// A clock that never waits, for worker.js in a timed run. A timer runs on the next task and moves
// performance.now on by its delay, so `play` ticks back to back and each tick is still one full
// TICK of simulated time. Import it before worker.js.
let now = performance.now();
let next = 1;
const queue = [];
const channel = new MessageChannel();
channel.port1.onmessage = () => {
  const timer = queue.shift();
  if (timer === undefined || timer.cancelled) return;
  now = Math.max(now, timer.at);
  timer.run();
};

performance.now = () => now;
globalThis.setTimeout = (run, ms = 0) => {
  const timer = { id: next++, at: now + Math.max(0, ms), run, cancelled: false };
  queue.push(timer);
  channel.port2.postMessage(null);
  return timer.id;
};
globalThis.clearTimeout = (id) => {
  const timer = queue.find((t) => t.id === id);
  if (timer !== undefined) timer.cancelled = true;
};
