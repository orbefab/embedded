import { lazy, Suspense, useState } from "react";
import { Stage } from "./stage";

const BlockView = lazy(() => import("./live").then((live) => ({ default: live.BlockView })));

export interface BlockPlayerProps {
  readonly blocks: readonly { readonly name: string; readonly title: string }[];
  readonly registry: string;
  readonly wasm: string;
}

/** The first Block live in 3D, with a tab for each other Block. Switching a tab frees the
 * Block before it. */
export function BlockPlayer({ blocks, registry, wasm }: BlockPlayerProps) {
  const [shown, setShown] = useState(blocks[0]?.name ?? "");
  return (
    <div>
      {blocks.length > 1 ? (
        <div className="mb-3 flex flex-wrap gap-2" role="tablist">
          {blocks.map((block) => (
            <button
              aria-selected={block.name === shown}
              className="rounded-md border border-border px-3 py-1.5 font-mono text-xs transition-colors hover:border-foreground aria-selected:border-brand aria-selected:text-brand"
              data-block={block.name}
              key={block.name}
              onClick={() => setShown(block.name)}
              role="tab"
              type="button"
            >
              {block.title}
            </button>
          ))}
        </div>
      ) : null}
      <Stage label="Loading the Block">
        {() => (
          <Suspense fallback={null}>
            <BlockView id={shown} key={shown} registry={registry} wasm={wasm} />
          </Suspense>
        )}
      </Stage>
    </div>
  );
}
