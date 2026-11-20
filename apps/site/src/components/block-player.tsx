import { lazy, type KeyboardEvent, type PointerEvent, Suspense, useEffect, useRef, useState } from "react";
import { captionOf } from "../captions";
import { holdGesture, type HoldGesture } from "./hold-gesture";
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
  const gesture = useRef<HoldGesture | null>(null);
  if (gesture.current === null) {
    gesture.current = holdGesture({
      press: () => hold.current?.press(),
      release: () => hold.current?.release(),
    });
  }

  useEffect(() => {
    const current = gesture.current;
    if (current === null) return;
    const onWindowBlur = () => current.drop();
    const onVisibility = () => {
      if (document.visibilityState === "hidden") current.drop();
    };
    // keyup and pointerup can land on another element after this control blurs.
    const onKeyUp = (event: globalThis.KeyboardEvent) => current.keyUp(event.key);
    const onPointerUp = (event: globalThis.PointerEvent) => current.pointerEnd(event.pointerId);
    window.addEventListener("blur", onWindowBlur);
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerUp);
    return () => {
      window.removeEventListener("blur", onWindowBlur);
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("pointercancel", onPointerUp);
      current.drop();
    };
  }, [hold]);

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== " " && event.key !== "Enter") return;
    event.preventDefault();
    gesture.current?.keyDown(event.key, event.repeat);
  };
  const onPointerDown = (event: PointerEvent<HTMLButtonElement>) => {
    gesture.current?.pointerDown(event.pointerId, event.button);
  };
  const onPointerLeave = (event: PointerEvent<HTMLButtonElement>) => {
    gesture.current?.pointerLeave(event.pointerId);
  };

  return (
    <button
      className="mt-3 touch-none border border-border px-3 py-1.5 text-sm hover:border-foreground focus-visible:[outline:2px_solid_var(--brand)] focus-visible:outline-offset-2"
      data-testid="hold-to-press"
      onBlur={() => gesture.current?.drop()}
      onClick={(event) => gesture.current?.click(event.nativeEvent.isTrusted)}
      onKeyDown={onKeyDown}
      onPointerDown={onPointerDown}
      onPointerLeave={onPointerLeave}
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
              className="border border-border px-3 py-1.5 font-mono text-xs transition-colors hover:border-foreground aria-selected:border-brand aria-selected:text-brand"
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
