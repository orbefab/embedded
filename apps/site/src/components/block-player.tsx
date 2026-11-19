import { lazy, type KeyboardEvent, type PointerEvent, Suspense, useEffect, useRef, useState } from "react";
import type { ButtonHold } from "./live";
import { Stage } from "./stage";

const BlockView = lazy(() => import("./live").then((live) => ({ default: live.BlockView })));

export interface BlockPlayerProps {
  readonly blocks: readonly { readonly name: string; readonly title: string }[];
  readonly registry: string;
  readonly wasm: string;
}

/** What a visitor can do with this Block, in plain words. Only a Block with its own button
 * takes a hold from the keyboard or from a touch on empty space. */
const CAPTION: Record<string, { readonly text: string; readonly button: boolean }> = {
  "nano-button-led": { text: "Press the button and the LED lights. Let go and it goes out.", button: true },
  "uno-button-led": { text: "Press the button and the LED lights, and the Uno's L LED with it. Let go and both go out.", button: true },
  "nano-led-fade": { text: "The LED fades up and down on its own.", button: false },
  "nano-light-comparator": { text: "Move the light slider. In the dark the LED lights.", button: false },
};

function interactive(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest("button, input, textarea, a") !== null;
}

/** The first Block live in 3D, with a tab for each other Block. Switching a tab frees the
 * Block before it. */
export function BlockPlayer({ blocks, registry, wasm }: BlockPlayerProps) {
  const [shown, setShown] = useState(blocks[0]?.name ?? "");
  const [webgl, setWebgl] = useState<boolean | null>(null);
  const hold = useRef<ButtonHold | null>(null);
  const holds = useRef(0);
  const keys = useRef(new Set<string>());
  const pointers = useRef(new Set<number>());
  const block = blocks.find((item) => item.name === shown) ?? blocks[0];
  const caption = (block && CAPTION[block.name]) ?? { text: block?.title ?? "", button: false };
  const pressable = caption.button && webgl === true;

  const begin = () => {
    holds.current += 1;
    if (holds.current === 1) hold.current?.press();
  };
  const end = () => {
    if (holds.current === 0) return;
    holds.current -= 1;
    if (holds.current === 0) hold.current?.release();
  };
  const dropHolds = () => {
    const held = holds.current > 0;
    holds.current = 0;
    keys.current.clear();
    pointers.current.clear();
    if (held) hold.current?.release();
  };

  useEffect(() => {
    dropHolds();
  }, [shown]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!pressable || interactive(event.target)) return;
    if (event.key !== " " && event.key !== "Enter") return;
    event.preventDefault();
    if (event.repeat || keys.current.has(event.key)) return;
    keys.current.add(event.key);
    begin();
  };
  const onKeyUp = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!keys.current.has(event.key)) return;
    keys.current.delete(event.key);
    end();
  };
  // A finger on a mesh stops at the viewer, which presses that part. A finger on empty
  // space bubbles here. A mouse click on empty space does not press the button.
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (!pressable || interactive(event.target) || event.button !== 0) return;
    if (event.pointerType !== "touch" && event.pointerType !== "pen") return;
    if (pointers.current.has(event.pointerId)) return;
    pointers.current.add(event.pointerId);
    begin();
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (!pointers.current.has(event.pointerId)) return;
    pointers.current.delete(event.pointerId);
    end();
  };

  return (
    <div>
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
      <div
        aria-label={
          pressable ? `${block?.title ?? "Block"}. ${caption.text} Hold Space or Enter, or touch the view, to press the button.` : undefined
        }
        className={
          pressable
            ? "rounded-lg outline-none focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand"
            : undefined
        }
        data-testid="block-stage"
        onBlur={pressable ? dropHolds : undefined}
        onKeyDown={pressable ? onKeyDown : undefined}
        onKeyUp={pressable ? onKeyUp : undefined}
        onPointerCancel={pressable ? onPointerUp : undefined}
        onPointerDown={pressable ? onPointerDown : undefined}
        onPointerUp={pressable ? onPointerUp : undefined}
        tabIndex={pressable ? 0 : undefined}
      >
        <Stage label="Loading the Block" onWebgl={setWebgl}>
          {() => (
            <Suspense fallback={null}>
              <BlockView hold={hold} id={shown} key={shown} registry={registry} wasm={wasm} />
            </Suspense>
          )}
        </Stage>
      </div>
      <p className="mt-3 text-muted-foreground text-sm" data-testid="block-caption">
        {caption.text}
      </p>
    </div>
  );
}
