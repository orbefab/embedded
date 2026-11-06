import { Viewer } from "@orbe/viewer";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { firstTarget, script } from "./first-target";
import { fixtureSource } from "./fixture-source";

// `?frame=idle|pressed|released` holds one frame of the script.
const held = new URLSearchParams(window.location.search).get("frame");
const hold = held !== null && held in script ? script[held as keyof typeof script] : undefined;
const source = fixtureSource(firstTarget, [script.idle, script.pressed, script.released], { stepMs: 2000, hold });

const root = document.getElementById("root");
if (root === null) throw new Error("index.html has no #root");
createRoot(root).render(
  <StrictMode>
    <Viewer
      onCreated={(state) => {
        Object.assign(window, { orbeViewer: state });
      }}
      source={source}
    />
  </StrictMode>
);
