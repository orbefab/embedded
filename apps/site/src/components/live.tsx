// The 3D views, in a chunk of their own that a page loads once it shows one, so a page without
// one never loads three.js.
import { loadBlock, type Ref, type SceneMessage, type SessionSource, Viewer, type ViewerProps, workerSource } from "@orbe/viewer";
import { useEffect, useState } from "react";

/** The canvas's live state, for the site's own browser checks, as the viewer page does. */
const expose: NonNullable<ViewerProps["onCreated"]> = (state) => Object.assign(window, { orbeViewer: { get: state.get } });

/** Press and release of the part the page named. */
export interface ButtonHold {
  press: () => void;
  release: () => void;
}

/** A Block from the registry, live in orbe-wasm's Worker, playing as soon as it opens. The
 * Worker and its object URLs are freed when the view unmounts or shows another Block. */
export function BlockView({
  id,
  registry,
  wasm,
  hold,
  press,
}: {
  id: string;
  registry: string;
  wasm: string;
  hold: { current: ButtonHold | null };
  /** The part the page named. The press waits until the scene is open. */
  press: Ref | null;
}) {
  const [source, setSource] = useState<SessionSource | null>(null);
  useEffect(() => {
    const base = new URL(wasm, document.baseURI);
    const opened = workerSource(
      () => new Worker(new URL("worker.js", base), { type: "module" }),
      () => loadBlock(new URL("index.json", new URL(registry, document.baseURI)), id)
    );
    let started = false;
    let button: Ref | null = null;
    let held = false;
    let want = false;
    const pressButton = () => {
      if (button === null || held) return;
      held = true;
      opened.send({ cmd: "press", ref: button }).catch(() => {
        held = false;
      });
    };
    const api: ButtonHold = {
      press() {
        want = true;
        pressButton();
      },
      release() {
        want = false;
        if (!held || button === null) return;
        const ref = button;
        held = false;
        opened.send({ cmd: "release", ref }).catch(() => {});
      },
    };
    const off = opened.subscribe((message) => {
      if (message.type === "scene") {
        // A new scene follows an open, which restarts the session and drops a held press.
        button = press;
        held = false;
        if (!started) {
          started = true;
          opened.send({ cmd: "play" }).catch(() => {});
        }
        if (want) pressButton();
      }
      if (message.type === "frame" && performance.getEntriesByName(`orbe:first-frame:${id}`).length === 0) {
        performance.mark(`orbe:first-frame:${id}`);
      }
    });
    hold.current = api;
    setSource(opened);
    return () => {
      api.release();
      if (hold.current === api) hold.current = null;
      off();
      opened.close();
    };
  }, [hold, id, press, registry, wasm]);
  return source === null ? null : <Viewer onCreated={expose} pageScroll source={source} />;
}

/** A source that shows one scene and runs nothing. */
function stillSource(scene: SceneMessage): SessionSource {
  return {
    subscribe(listener) {
      listener(scene);
      return () => {};
    },
    send: () => Promise.reject(new Error("this page shows the part with no circuit; a Block's page runs one")),
    close() {},
  };
}

/** One part on its own, from the scene message the build wrote with orbe-wasm's `partScene`. */
export function PartView({ url }: { url: string }) {
  const [source, setSource] = useState<SessionSource | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let current = true;
    fetch(url)
      .then((response) => (response.ok ? response.json() : Promise.reject(new Error("load"))))
      .then((scene: SceneMessage) => current && setSource(stillSource(scene)))
      .catch(() => current && setFailed(true));
    return () => {
      current = false;
    };
  }, [url]);
  if (failed) return <p className="p-6 font-mono text-destructive text-sm">The 3D model could not load.</p>;
  return source === null ? null : <Viewer onCreated={expose} pageScroll source={source} />;
}
