import { Wordmark } from "@orbe/brand/mark";
import type { ReactNode } from "react";
import { at } from "../paths";
import type { Site } from "../records";

export function Header() {
  return (
    <header className="border-border border-b">
      <div className="mx-auto flex max-w-5xl flex-wrap items-center justify-between gap-4 px-6 py-4">
        <a className="flex items-center gap-3" href={at.home}>
          <Wordmark className="text-[1.25rem]" />
          <span className="font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">embedded</span>
        </a>
        <nav className="flex items-center gap-5 font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">
          <a className="transition-colors hover:text-foreground" href={at.parts}>
            Parts
          </a>
          <a className="transition-colors hover:text-foreground" href={at.blocks}>
            Blocks
          </a>
          <a className="transition-colors hover:text-foreground" href={at.website}>
            Orbefab
          </a>
        </nav>
      </div>
    </header>
  );
}

export function Footer({ site }: { site: Site }) {
  return (
    <footer className="mt-24 border-border border-t">
      <div className="mx-auto grid max-w-5xl gap-3 px-6 py-10 text-muted-foreground text-sm">
        <p>
          orbe and this catalog are open source under MIT OR Apache-2.0, unless a file says otherwise. Each file lists its own license.
          {__ORBE_SITE_INSTALL__ ? (
            <>
              {" "}
              The source is at{" "}
              <a className="underline hover:text-foreground" href="https://github.com/orbefab/embedded">
                github.com/orbefab/embedded
              </a>
              .
            </>
          ) : null}
        </p>
        <p>
          Maker names on these pages say which real part a model matches. Orbefab is not affiliated with or endorsed by those makers, and the models
          carry no logos.
        </p>
        <p className="font-mono text-xs">
          Catalog {site.registry} at{" "}
          <a className="underline hover:text-foreground" href={at.index}>
            {at.index}
          </a>
        </p>
      </div>
    </footer>
  );
}

export function Shell({ site, children }: { site: Site; children: ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col">
      <Header />
      <main className="mx-auto w-full max-w-5xl flex-1 px-6 pt-12">{children}</main>
      <Footer site={site} />
    </div>
  );
}

export function Section({ title, id, children }: { title: string; id?: string; children: ReactNode }) {
  return (
    <section className="mt-14" id={id}>
      <h2 className="font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">{title}</h2>
      <div className="mt-4">{children}</div>
    </section>
  );
}

export function Code({ children }: { children: ReactNode }) {
  return <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-[0.9em]">{children}</code>;
}

/** Says the CLI is coming, where install lines show once it is public, so a page never shows a
 * command that fails. */
export function CliComing() {
  return (
    <p className="text-muted-foreground" data-testid="cli-coming">
      The orbe CLI is coming. Its source and binaries are not public yet, so there is nothing to install today. Until then, browse the parts and play
      the Blocks here.
    </p>
  );
}

/** Install lines. Each page shows them only under `__ORBE_SITE_INSTALL__`, so a build without
 * the flag holds none of them. */
export function Install({ lines, note }: { lines: readonly string[]; note?: ReactNode }) {
  return (
    <div data-testid="install">
      <pre className="overflow-x-auto rounded-lg border border-border bg-muted p-4 font-mono text-sm">{lines.join("\n")}</pre>
      {note === undefined ? null : <p className="mt-3 text-muted-foreground text-sm">{note}</p>}
    </div>
  );
}

/** A table with a header row, its cells given as rows. */
export function Table({ head, rows }: { head: readonly string[]; rows: readonly (readonly ReactNode[])[] }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full border-collapse text-left text-sm">
        <thead>
          <tr className="border-border border-b font-mono text-muted-foreground text-xs uppercase tracking-[0.12em]">
            {head.map((cell) => (
              <th className="py-2 pr-6 font-normal" key={cell}>
                {cell}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
            <tr className="border-border border-b align-top" key={index}>
              {row.map((cell, column) => (
                <td className="py-2 pr-6" key={column}>
                  {cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Absent data, shown as absent and never made up. */
export function None({ children }: { children: ReactNode }) {
  return <p className="text-muted-foreground">{children}</p>;
}
