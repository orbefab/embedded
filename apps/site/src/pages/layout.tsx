import { Wordmark } from "@orbe/brand/mark";
import { useRouterState } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { CopyButton } from "../components/copy-button";
import { ThemeToggle } from "../components/theme-toggle";
import { at, sitePath } from "../paths";

type Section = "blocks" | "parts" | "get";

function sectionOf(pathname: string): Section | null {
  const path = sitePath(pathname);
  if (/^\/blocks\/[^/]+\/embed\/?$/.test(path)) return null;
  if (path === "/blocks" || path.startsWith("/blocks/")) return "blocks";
  if (path === "/parts" || path.startsWith("/parts/")) return "parts";
  if (path === "/" || path === "") return "get";
  return null;
}

function NavLink({ href, label, current, accent }: { href: string; label: string; current: boolean; accent?: boolean }) {
  return (
    <a
      aria-current={current ? "page" : undefined}
      className={`relative inline-flex min-h-11 items-center px-3 font-mono text-[0.6875rem] uppercase tracking-[0.16em] hover:text-foreground ${
        current ? "text-foreground" : accent ? "text-brand" : "text-muted-foreground"
      }`}
      href={href}
    >
      {label}
      {current ? <span className="absolute inset-x-3 bottom-2 h-0.5 bg-brand" /> : null}
    </a>
  );
}

/** The header both sites share. The wordmark goes to the Orbefab home. Get orbe is the one
 * orange link. The open section keeps an orange underline. On a phone the links drop to a
 * second row. */
export function Header() {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const section = sectionOf(pathname);
  return (
    <header className="sticky top-0 z-50 border-border border-b bg-[color-mix(in_srgb,var(--background)_82%,transparent)] backdrop-blur-[8px]">
      <div className="mx-auto flex max-w-7xl items-center justify-between gap-4 px-6 py-3 max-sm:flex-wrap max-sm:gap-y-0 max-sm:py-1.5">
        <a className="inline-flex min-h-11 min-w-0 items-center max-sm:order-1 max-sm:flex-1" href="/">
          <Wordmark className="text-[1.35rem]" />
        </a>
        <div className="flex items-center gap-1 max-sm:contents">
          <nav aria-label="Main" className="flex items-center max-sm:order-3 max-sm:-mx-6 max-sm:mt-1.5 max-sm:w-[calc(100%+3rem)] max-sm:border-border max-sm:border-t max-sm:px-3">
            <NavLink current={section === "blocks"} href={at.blocks} label="Blocks" />
            <NavLink current={section === "parts"} href={at.parts} label="Parts" />
            <NavLink accent current={section === "get"} href={at.home} label="Get orbe" />
          </nav>
          <ThemeToggle className="max-sm:order-2" />
        </div>
      </div>
    </header>
  );
}

const FOOTER_LINK =
  "inline-flex min-h-[1.9rem] items-center text-sm text-[color-mix(in_srgb,var(--foreground)_80%,transparent)] hover:text-foreground";

function FooterLink({ href, children }: { href: string; children: string }) {
  return (
    <li>
      <a className={FOOTER_LINK} href={href}>
        {children}
      </a>
    </li>
  );
}

const footerLabel = "font-mono text-[0.6875rem] text-muted-foreground uppercase tracking-[0.16em]";

/** The footer both sites share. */
export function Footer({ className = "mt-24" }: { className?: string }) {
  return (
    <footer className={`${className} border-border border-t`}>
      <div className="mx-auto grid max-w-7xl grid-cols-2 gap-x-8 gap-y-8 px-6 pt-10 pb-10 md:grid-cols-[minmax(0,1.4fr)_minmax(0,0.6fr)_minmax(0,0.6fr)] md:gap-x-12">
        <div className="col-span-2 grid max-w-[34rem] content-start gap-2.5 text-[0.8125rem] text-muted-foreground md:col-span-1">
          <p className="font-mono text-foreground text-xs">© 2026 Orbefab</p>
          <p>Open source under MIT or Apache-2.0.</p>
        </div>
        <div>
          <p className={footerLabel}>orbe</p>
          <ul className="mt-2.5 grid gap-0.5">
            <FooterLink href={at.home}>Get orbe</FooterLink>
            <FooterLink href={at.blocks}>Blocks</FooterLink>
            <FooterLink href={at.parts}>Parts</FooterLink>
            <FooterLink href="https://github.com/orbefab/orbe/releases">Releases</FooterLink>
            <FooterLink href="https://github.com/orbefab/orbe">GitHub</FooterLink>
          </ul>
        </div>
        <div>
          <p className={footerLabel}>Orbefab</p>
          <ul className="mt-2.5 grid gap-0.5">
            <FooterLink href="/">Home</FooterLink>
            <FooterLink href="/brand">Brand</FooterLink>
            <FooterLink href="https://github.com/orbefab/orbe/blob/main/LICENSE-MIT">License</FooterLink>
          </ul>
        </div>
      </div>
    </footer>
  );
}

export function Shell({ children, missing = false }: { children: ReactNode; missing?: boolean }) {
  return (
    <div className="flex min-h-screen flex-col">
      <Header />
      <main
        className={
          missing
            ? "mx-auto flex w-full min-w-0 max-w-7xl flex-1 flex-col justify-center px-6 py-16"
            : "mx-auto w-full min-w-0 max-w-5xl flex-1 px-6 pt-12"
        }
      >
        {children}
      </main>
      <Footer className={missing ? "mt-auto" : "mt-24"} />
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

/** Install lines, with one Copy button for the whole block. Each page shows them only under
 * `__ORBE_SITE_INSTALL__`, so a build without the flag holds none of them. */
export function Install({ lines, note }: { lines: readonly string[]; note?: ReactNode }) {
  const text = lines.join("\n");
  return (
    <div data-copy-root data-testid="install">
      <div className="flex min-w-0 items-stretch overflow-hidden border border-border bg-muted font-mono text-sm">
        <pre className="min-w-0 flex-1 overflow-x-auto p-4" data-copy-text>{text}</pre>
        <CopyButton boxed text={text} />
      </div>
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

