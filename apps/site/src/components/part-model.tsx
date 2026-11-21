import { lazy, Suspense, useState } from "react";
import { Stage } from "./stage";

const PartView = lazy(() => import("./live").then((live) => ({ default: live.PartView })));

/** The part live in 3D. `shown` says which param values the view draws, and hides with the view
 * when the browser has no WebGL 2. */
export function PartModel({ url, shown }: { url: string; shown?: string | undefined }) {
  const [webgl, setWebgl] = useState<boolean | null>(null);
  return (
    <>
      <Stage label="Loading the 3D model" onWebgl={setWebgl}>
        {() => (
          <Suspense fallback={null}>
            <PartView url={url} />
          </Suspense>
        )}
      </Stage>
      {shown === undefined || webgl === false ? null : (
        <p className="mt-3 text-muted-foreground text-sm" data-testid="shown-at">
          {shown}
        </p>
      )}
    </>
  );
}
