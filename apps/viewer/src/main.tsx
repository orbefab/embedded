import { type SessionSource, sessionUrl, Viewer, websocketSource } from "@orbe/viewer";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { FloatType, WebGLRenderTarget } from "three";
import { firstTarget, script } from "./first-target";
import { fixtureSource } from "./fixture-source";

/** The live session when `orbe show` served the page, which puts a token in the URL's
 * fragment, else the first target's fixture. */
function pickSource(): SessionSource {
  const url = sessionUrl(window.location);
  if (url !== null) return websocketSource(() => new WebSocket(url));
  // `?frame=idle|pressed|released` holds one frame of the script.
  const held = new URLSearchParams(window.location.search).get("frame");
  const hold = held !== null && held in script ? script[held as keyof typeof script] : undefined;
  const source = fixtureSource(firstTarget, [script.idle, script.pressed, script.released], { stepMs: 2000, hold });
  Object.assign(window, { orbeFixture: { source, firstTarget, script, three: { FloatType, WebGLRenderTarget } } });
  return source;
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
