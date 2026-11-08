import { lazy, Suspense } from "react";
import { Stage } from "./stage";

const PartView = lazy(() => import("./live").then((live) => ({ default: live.PartView })));

export function PartModel({ url }: { url: string }) {
  return (
    <Stage label="Loading the 3D model">
      {() => (
        <Suspense fallback={null}>
          <PartView url={url} />
        </Suspense>
      )}
    </Stage>
  );
}
