import { loadBlock, type SessionSource, sessionUrl, Viewer, websocketSource, workerSource } from "@orbe/viewer";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

/** The live session when `orbe show` served the page, which puts a token in the URL's
 * fragment. Otherwise a Block from a registry, run in orbe-wasm's Worker, the same path
 * `?block=<id>` uses. With no `block`, the page runs `nano-button-led`. `registry` names the
 * registry's folder and `wasm` the package's, by default `registry/` and `orbe-wasm/` beside
 * the page. */
function pickSource(): SessionSource {
  const url = sessionUrl(window.location);
  if (url !== null) return websocketSource(() => new WebSocket(url));
  const params = new URLSearchParams(window.location.search);
  const block = params.get("block") ?? "nano-button-led";
  const registry = new URL(params.get("registry") ?? "registry/", document.baseURI);
  const wasm = new URL(params.get("wasm") ?? "orbe-wasm/", document.baseURI);
  return workerSource(
    () => new Worker(new URL("worker.js", wasm), { type: "module" }),
    () => loadBlock(new URL("index.json", registry), block)
  );
}

const source = pickSource();
const root = document.getElementById("root");
if (root === null) throw new Error("index.html has no #root");
createRoot(root).render(
  <StrictMode>
    <Viewer
      onCreated={(state) => {
        // `get` returns the live state. The state passed here keeps the startup camera.
        Object.assign(window, { orbeViewer: { get: state.get } });
      }}
      source={source}
    />
  </StrictMode>
);
