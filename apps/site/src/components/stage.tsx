import { type ReactNode, useEffect, useState } from "react";

/** The box a 3D view draws in. The server renders it empty, and the browser fills it once the
 * page hydrates, so the page's text never waits for three.js. */
export function Stage({ label, children }: { label: string; children: () => ReactNode }) {
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);
  return (
    <div className="relative h-[min(62vh,560px)] overflow-hidden rounded-lg border border-border bg-[#1b1d21]">
      <p className="absolute inset-0 grid place-items-center font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">
        {label}
      </p>
      {mounted ? <div className="absolute inset-0">{children()}</div> : null}
    </div>
  );
}
