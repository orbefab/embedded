import { PartSearch } from "../components/part-search";
import type { Listed } from "../data";

export function PartsPage({ parts }: { parts: readonly Listed[] }) {
  return (
    <>
      <h1 className="font-semibold text-3xl tracking-tight md:text-4xl">Parts</h1>
      <p className="mt-3 max-w-2xl text-muted-foreground text-lg">
        Each part is a real part, with its pins, its params, its ratings and their sources, what the engine leaves out, its 3D model and every file
        under its licence. The search finds what <code className="font-mono">orbe search</code> finds.
      </p>
      <div className="mt-10">
        <PartSearch parts={parts} />
      </div>
    </>
  );
}
