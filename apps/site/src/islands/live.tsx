// The 3D views, in a chunk of their own that a page loads once it shows one, so a page without
// one never loads three.js.
import { loadBlock, type SceneMessage, type SessionSource, Viewer, type ViewerProps, workerSource } from "@orbe/viewer";
import { useEffect, useState } from "react";

/** The canvas's live state, for the site's own browser checks, as the viewer page does. */
const expose: NonNullable<ViewerProps["onCreated"]> = (state) => Object.assign(window, { orbeViewer: { get: state.get } });

/** A Block from the registry, live in orbe-wasm's Worker, playing as soon as it opens. The
 * Worker and its object URLs are freed when the view unmounts or shows another Block. */
export function BlockView({ id, registry, wasm }: { id: string; registry: string; wasm: string }) {
  const [source, setSource] = useState<SessionSource | null>(null);
  useEffect(() => {
    const base = new URL(wasm, document.baseURI);
    const opened = workerSource(
      () => new Worker(new URL("worker.js", base), { type: "module" }),
      () => loadBlock(new URL("index.json", new URL(registry, document.baseURI)), id)
    );
    let started = false;
    const off = opened.subscribe((message) => {
      if (message.type === "scene" && !started) {
        started = true;
        opened.send({ cmd: "play" }).catch(() => {});
      }
      if (message.type === "frame" && performance.getEntriesByName(`orbe:first-frame:${id}`).length === 0) {
        performance.mark(`orbe:first-frame:${id}`);
      }
    });
    setSource(opened);
    return () => {
      off();
      opened.close();
    };
  }, [id, registry, wasm]);
  return source === null ? null : <Viewer onCreated={expose} source={source} />;
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
  const [failed, setFailed] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    fetch(url)
      .then((response) => (response.ok ? response.json() : Promise.reject(new Error(`${url} answered ${response.status}`))))
      .then((scene: SceneMessage) => current && setSource(stillSource(scene)))
      .catch((error: unknown) => current && setFailed(error instanceof Error ? error.message : String(error)));
    return () => {
      current = false;
    };
  }, [url]);
  if (failed !== null) return <p className="p-6 font-mono text-destructive text-sm">The model did not load: {failed}</p>;
  return source === null ? null : <Viewer onCreated={expose} source={source} />;
}
