import { PartSearch } from "../components/part-search";
import type { Listed } from "../data";

export function PartsPage({ parts }: { parts: readonly Listed[] }) {
  return (
    <>
      <h1 className="font-semibold text-3xl tracking-tight md:text-4xl">Parts</h1>
      <p className="mt-3 max-w-2xl text-muted-foreground text-lg">
        Real parts with their pins, ratings and 3D models.
      </p>
      <div className="mt-10">
        <PartSearch parts={parts} />
      </div>
    </>
  );
}
