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
          className="mt-2 w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-sm outline-none focus:border-brand"
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
      <ul className="mt-4 divide-y divide-border border-border border-y">
        {shown.map((part) => (
          <li data-part={part.name} key={part.name}>
            <a className="grid gap-1 py-4 transition-colors hover:text-brand sm:grid-cols-[minmax(0,1fr)_auto]" href={part.href}>
              <span>
                <span className="font-medium">{part.title}</span>
                <span className="block text-muted-foreground text-sm">
                  {part.description ?? "No summary yet."}
                </span>
              </span>
              <span className="font-mono text-muted-foreground text-xs sm:text-right">
                {part.name}
                {part.maker || part.number ? (
                  <span className="block">{[part.maker, part.number].filter(Boolean).join(" ")}</span>
                ) : null}
              </span>
            </a>
          </li>
        ))}
      </ul>
    </div>
  );
}
