import type { Catalog } from "../catalog";
import { Island } from "../islands";
import type { Listed } from "../islands/part-search";
import { at } from "../paths";
import type { PartRecord } from "../records";

const listed = (record: PartRecord): Listed => ({
  name: record.name,
  title: record.title,
  maker: record.real.maker,
  number: record.real.number,
  description: record.description,
  categories: record.categories,
  href: at.part(record.name),
});

export function PartsPage({ catalog }: { catalog: Catalog }) {
  const parts = [...catalog.parts].sort((a, b) => a.record.title.localeCompare(b.record.title)).map(({ record }) => listed(record));
  return (
    <>
      <h1 className="font-semibold text-3xl tracking-tight md:text-4xl">Parts</h1>
      <p className="mt-3 max-w-2xl text-muted-foreground text-lg">
        Each part is a real part, with its pins, its params, its ratings and their sources, what the engine leaves out, its 3D model and every file
        under its licence. The search finds what <code className="font-mono">orbe search</code> finds.
      </p>
      <div className="mt-10">
        <Island name="part-search" props={{ parts }} />
      </div>
    </>
  );
}
