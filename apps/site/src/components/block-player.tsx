import { lazy, type KeyboardEvent, type PointerEvent, Suspense, useEffect, useRef, useState } from "react";
import { captionOf } from "../captions";
import type { ButtonHold } from "./live";
import { Stage } from "./stage";

const BlockView = lazy(() => import("./live").then((live) => ({ default: live.BlockView })));

export interface BlockPlayerProps {
  readonly blocks: readonly { readonly name: string; readonly title: string; readonly button: boolean }[];
  readonly registry: string;
  readonly wasm: string;
  /** Fill the frame: the stage grows, and the caption and hold control sit under it. */
  readonly fill?: boolean;
}

/** Holds the circuit's button while a pointer, Space or Enter is down on this control, and
 * releases when that gesture ends, the control blurs, the window blurs, or the page is hidden. */
function PressControl({ hold }: { hold: { readonly current: ButtonHold | null } }) {
  const down = useRef(0);
  const keys = useRef(new Set<string>());
  const pointers = useRef(new Set<number>());
  // A key or pointer gesture also fires a click when it ends; that click is not a second press.
  const gesture = useRef(false);

  const press = () => {
    down.current += 1;
    if (down.current === 1) hold.current?.press();
  };
  const releaseOne = () => {
    if (down.current === 0) return;
    down.current -= 1;
    if (down.current === 0) hold.current?.release();
  };
  const drop = () => {
    const held = down.current > 0;
    down.current = 0;
    keys.current.clear();
    pointers.current.clear();
    gesture.current = false;
    if (held) hold.current?.release();
  };

  useEffect(() => {
    const onWindowBlur = () => drop();
    const onVisibility = () => {
      if (document.visibilityState === "hidden") drop();
    };
    window.addEventListener("blur", onWindowBlur);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      window.removeEventListener("blur", onWindowBlur);
      document.removeEventListener("visibilitychange", onVisibility);
      drop();
    };
  }, [hold]);

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== " " && event.key !== "Enter") return;
    event.preventDefault();
    if (event.repeat || keys.current.has(event.key)) return;
    keys.current.add(event.key);
    gesture.current = true;
    press();
  };
  const onKeyUp = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (!keys.current.delete(event.key)) return;
    endGesture();
    releaseOne();
  };
  const onPointerDown = (event: PointerEvent<HTMLButtonElement>) => {
    if (event.button !== 0 || pointers.current.has(event.pointerId)) return;
    pointers.current.add(event.pointerId);
    gesture.current = true;
    press();
  };
  const onPointerEnd = (event: PointerEvent<HTMLButtonElement>) => {
    if (!pointers.current.delete(event.pointerId)) return;
    endGesture();
    releaseOne();
  };
  const endGesture = () => {
    setTimeout(() => {
      gesture.current = false;
    }, 0);
  };
  // A click with no key or pointer gesture before it, as a screen reader sends, presses briefly.
  const onClick = () => {
    if (gesture.current || down.current > 0) return;
    press();
    setTimeout(releaseOne, 300);
  };

  return (
    <button
      className="mt-3 touch-none rounded-md border border-border px-3 py-1.5 text-sm outline-none hover:border-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand"
      data-testid="hold-to-press"
      onBlur={drop}
      onClick={onClick}
      onKeyDown={onKeyDown}
      onKeyUp={onKeyUp}
      onPointerCancel={onPointerEnd}
      onPointerDown={onPointerDown}
      onPointerLeave={onPointerEnd}
      onPointerUp={onPointerEnd}
      type="button"
    >
      Hold to press
    </button>
  );
}

/** The first Block live in 3D, with a tab for each other Block. Switching a tab frees the
 * Block before it. A Block whose scene places a part with a press input gets the hold control. */
export function BlockPlayer({ blocks, registry, wasm, fill = false }: BlockPlayerProps) {
  const [shown, setShown] = useState(blocks[0]?.name ?? "");
  const [webgl, setWebgl] = useState<boolean | null>(null);
  const hold = useRef<ButtonHold | null>(null);
  const block = blocks.find((item) => item.name === shown) ?? blocks[0];
  const name = block?.name ?? "";
  const caption = name === "" ? "" : captionOf(name);
  const pressable = block?.button === true && webgl === true;

  return (
    <div className={fill ? "flex h-full min-h-0 min-w-0 flex-col" : undefined}>
      {blocks.length > 1 ? (
        <div className="mb-3 flex flex-wrap gap-2" role="tablist">
          {blocks.map((item) => (
            <button
              aria-selected={item.name === shown}
              className="rounded-md border border-border px-3 py-1.5 font-mono text-xs transition-colors hover:border-foreground aria-selected:border-brand aria-selected:text-brand"
              data-block={item.name}
              key={item.name}
              onClick={() => setShown(item.name)}
              role="tab"
              type="button"
            >
              {item.title}
            </button>
          ))}
        </div>
      ) : null}
      <div className={fill ? "min-h-0 min-w-0 flex-1" : undefined} data-testid="block-stage">
        <Stage fill={fill} label="Loading the Block" onWebgl={setWebgl}>
          {() => (
            <Suspense fallback={null}>
              <BlockView hold={hold} id={shown} key={shown} registry={registry} wasm={wasm} />
            </Suspense>
          )}
        </Stage>
      </div>
      {webgl === false ? null : (
        <p className={`text-muted-foreground text-sm ${fill ? "shrink-0 px-3 pt-3 break-words" : "mt-3"}`} data-testid="block-caption">
          {caption}
        </p>
      )}
      {pressable ? (
        <div className={fill ? "shrink-0 px-3 pb-3" : undefined} key={shown}>
          <PressControl hold={hold} />
        </div>
      ) : null}
    </div>
  );
}
