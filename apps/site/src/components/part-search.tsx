import { useEffect, useState } from "react";
import type { Listed } from "../data";
import { search } from "../search";
import { groupParts } from "../sections";

function PartGrid({ parts }: { parts: readonly Listed[] }) {
  return (
    <ul className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {parts.map((part) => (
        <li data-part={part.name} key={part.name}>
          <a
            className="flex h-full min-w-0 flex-col gap-1 border border-border p-4 transition-colors hover:border-brand max-sm:flex-row max-sm:items-center max-sm:justify-between max-sm:gap-3"
            href={part.href}
          >
            <span className="min-w-0 font-medium">{part.title}</span>
            {part.use === "" ? null : <span className="font-mono text-foreground text-xs sm:mt-auto">{part.use}</span>}
          </a>
        </li>
      ))}
    </ul>
  );
}

/** Every part grouped into sections, and a box that finds parts across all sections as
 * `orbe search` does. */
export function PartSearch({ parts }: { parts: readonly Listed[] }) {
  const [query, setQuery] = useState("");
  useEffect(() => setQuery(new URLSearchParams(window.location.search).get("q") ?? ""), []);
  const searching = query.trim() !== "";
  const shown = searching ? search(parts, query) : parts;
  const groups = searching ? [] : groupParts(parts);
  const change = (text: string) => {
    setQuery(text);
    const url = new URL(window.location.href);
    if (text.trim() === "") url.searchParams.delete("q");
    else url.searchParams.set("q", text);
    window.history.replaceState(null, "", url);
  };
  return (
    <div>
      <label className="block">
        <span className="font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">Search</span>
        <input
          className="mt-2 w-full border border-input bg-transparent px-3 py-2 font-mono text-sm outline-none focus:border-brand"
          name="q"
          onChange={(event) => change(event.target.value)}
          placeholder="led, 0603, kingbright, board"
          type="search"
          value={query}
        />
      </label>
      <p className="mt-3 font-mono text-muted-foreground text-xs" data-testid="count">
        {shown.length === parts.length ? `${parts.length} parts` : `${shown.length} of ${parts.length} parts`}
      </p>
      {searching ? (
        <PartGrid parts={shown} />
      ) : (
        <div>
          {groups.map((group) => (
            <section className="mt-10 first:mt-6" data-section={group.section} key={group.section}>
              <div className="flex items-baseline justify-between gap-4 border-border border-b pb-2">
                <h2 className="font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">{group.section}</h2>
                <span className="font-mono text-muted-foreground text-xs">{group.parts.length}</span>
              </div>
              <PartGrid parts={group.parts} />
            </section>
          ))}
        </div>
      )}
    </div>
  );
}
