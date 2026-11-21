import { useEffect, useState } from "react";
import type { Listed } from "../data";
import { search } from "../search";

/** Every part, and a box that finds parts as `orbe search` does. */
export function PartSearch({ parts }: { parts: readonly Listed[] }) {
  const [query, setQuery] = useState("");
  useEffect(() => setQuery(new URLSearchParams(window.location.search).get("q") ?? ""), []);
  const shown = query.trim() === "" ? parts : search(parts, query);
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
      <ul className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {shown.map((part) => (
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
    </div>
  );
}
