import type { ReactNode } from "react";
import type { BlockCard, Linked } from "../data";
import { at } from "../paths";
import { CliComing, Install } from "./layout";

export interface Home {
  readonly blocks: readonly BlockCard[];
  readonly boards: readonly Linked[];
  readonly parts: number;
  /** nano-led-fade's scene.jsonc, the file the third step shows. */
  readonly scene: string;
}

const STEP = "grid grid-cols-1 gap-2 border-border border-t py-7 sm:grid-cols-[4rem_minmax(0,1fr)] sm:gap-x-4";

function Step({ n, title, children }: { n: string; title: string; children: ReactNode }) {
  return (
    <li className={STEP}>
      <span className="font-mono text-brand text-sm">{n}</span>
      <div className="grid min-w-0 max-w-[44rem] grid-cols-1 gap-3">
        <h2 className="font-medium text-xl">{title}</h2>
        {children}
      </div>
    </li>
  );
}

function SceneFile({ text }: { text: string }) {
  const lines = text.replace(/\n$/, "").split("\n");
  return (
    <div className="min-w-0 overflow-hidden border border-border bg-muted font-mono text-sm" data-testid="scene-file">
      <div className="flex justify-between gap-4 border-border border-b px-4 py-2 text-muted-foreground text-xs">
        <span>scene.jsonc</span>
        <span>nano-led-fade</span>
      </div>
      <pre className="overflow-x-auto p-4 leading-6">
        {lines.map((line, index) => {
          const cut = line.indexOf("//");
          return (
            <div key={index}>
              <span className="mr-4 inline-block w-6 select-none text-right text-muted-foreground">{index + 1}</span>
              {cut < 0 ? (
                line
              ) : (
                <>
                  {line.slice(0, cut)}
                  <span className="text-muted-foreground">{line.slice(cut)}</span>
                </>
              )}
            </div>
          );
        })}
      </pre>
    </div>
  );
}

function Boards({ boards }: { boards: readonly Linked[] }) {
  const sorted = [...boards].sort((a, b) => a.title.localeCompare(b.title));
  return (
    <p className="mt-8 text-muted-foreground">
      Boards today:{" "}
      {sorted.map((board, index) => (
        <span key={board.href}>
          {index === 0 ? "" : index === sorted.length - 1 ? " and " : ", "}
          <a className="underline underline-offset-[3px] hover:text-brand" href={board.href}>
            {board.title}
          </a>
        </span>
      ))}
      .
    </p>
  );
}

/** Get orbe. The home of orbefab.com is the entry; this page is how you install and run it. */
export function HomePage({ home }: { home: Home }) {
  return (
    <>
      <p className="font-mono text-[0.6875rem] text-brand uppercase tracking-[0.16em]">Get orbe</p>
      <h1 className="mt-3 max-w-[20ch] font-medium text-[clamp(2rem,4vw,3rem)] leading-[1.1] tracking-tight">
        A hardware simulator for the AI agent you already use.
      </h1>
      <p className="mt-4 max-w-[42rem] text-[1.125rem] text-muted-foreground leading-relaxed">
        orbe runs real firmware on simulated Arduino boards, so your agent can test a circuit before you build it.
      </p>
      <div className="mt-8 grid border-border border-y sm:grid-cols-3">
        <div className="py-4 sm:pr-6">
          <p className="flex items-center gap-2 font-medium">
            <span className="inline-block size-[0.45rem] bg-brand" />
            Any agent
          </p>
          <p className="mt-2 text-[0.9375rem] text-muted-foreground leading-normal">Use the AI agent you already have, on your own subscription.</p>
        </div>
        <div className="border-border py-4 max-sm:border-t sm:border-l sm:px-6">
          <p className="flex items-center gap-2 font-medium">
            <span className="inline-block size-[0.45rem] bg-brand" />
            Agent first
          </p>
          <p className="mt-2 text-[0.9375rem] text-muted-foreground leading-normal">
            Every step is a command an agent can run, and <code className="font-mono">orbe skills get core</code> tells it how.
          </p>
        </div>
        <div className="border-border py-4 max-sm:border-t sm:border-l sm:pl-6">
          <p className="flex items-center gap-2 font-medium">
            <span className="inline-block size-[0.45rem] bg-brand" />
            Headless or live
          </p>
          <p className="mt-2 text-[0.9375rem] text-muted-foreground leading-normal">
            In a terminal for the agent, or live in 3D for you, on every Block page.
          </p>
        </div>
      </div>

      <ol className="mt-14 list-none border-border border-b">
        <Step n="01" title="Install orbe">
          {__ORBE_SITE_INSTALL__ ? (
            <Install
              lines={["curl -fsSL https://orbefab.com/embedded/install.sh | sh"]}
              note={
                <>
                  The script installs a prebuilt <code className="font-mono">orbe</code> with the viewer, for Linux x86_64 and macOS arm64, into ~/.local/bin.
                  The archives are on the{" "}
                  <a className="underline hover:text-foreground" href="https://github.com/orbefab/embedded/releases">
                    releases page
                  </a>
                  .
                </>
              }
            />
          ) : (
            <CliComing />
          )}
        </Step>
        <Step n="02" title="Tell your agent">
          {__ORBE_SITE_INSTALL__ ? (
            <>
              <Install lines={["orbe skills get core"]} />
              <p className="text-muted-foreground">It prints how an agent uses orbe. Your agent can run it, or you can read it first.</p>
            </>
          ) : (
            <CliComing />
          )}
        </Step>
        <Step n="03" title="Start from a Block">
          {__ORBE_SITE_INSTALL__ ? (
            <>
              <p className="text-muted-foreground">
                A Block is a small circuit that works: a scene file, its firmware and its tests. In an empty folder, <code className="font-mono">orbe add</code>{" "}
                copies one in.
              </p>
              <Install lines={["orbe add nano-led-fade"]} />
              <p className="text-muted-foreground text-sm">
                It copies the parts into <code className="font-mono">parts/</code>, records them in <code className="font-mono">parts.lock</code>, then writes{" "}
                <code className="font-mono">scene.jsonc</code>, <code className="font-mono">firmware/</code> and <code className="font-mono">tests/</code>. The
                scene file describes the circuit:
              </p>
              <SceneFile text={home.scene} />
            </>
          ) : (
            <CliComing />
          )}
        </Step>
        <Step n="04" title="Check it and test it">
          {__ORBE_SITE_INSTALL__ ? (
            <>
              <Install lines={["orbe check scene.jsonc"]} />
              <Install lines={["orbe test"]} />
              <p className="text-muted-foreground">
                orbe flashes the real firmware onto the simulated Nano and runs the Block&apos;s tests, headless. Your agent can run the same two commands after
                each change.
              </p>
            </>
          ) : (
            <CliComing />
          )}
        </Step>
      </ol>

      <div className="mt-14 grid gap-4 sm:grid-cols-2">
        <a className="block border border-border p-5 transition-colors hover:border-brand" href={at.blocks}>
          <span className="font-medium">Blocks →</span>
          <span className="mt-2 block text-muted-foreground text-sm">
            {home.blocks.length} small circuits that work. Each one plays live in 3D in your browser.
          </span>
        </a>
        <a className="block border border-border p-5 transition-colors hover:border-brand" href={at.parts}>
          <span className="font-medium">Parts →</span>
          <span className="mt-2 block text-muted-foreground text-sm">{home.parts} parts in the catalog, from boards to resistors.</span>
        </a>
      </div>
      <Boards boards={home.boards} />
    </>
  );
}

export function NotFoundPage() {
  return (
    <>
      <div className="flex items-center gap-4">
        <p className="font-mono text-[0.625rem] text-brand uppercase tracking-[0.22em]">404 NOT FOUND</p>
        <span className="h-px max-w-24 flex-1 bg-border" />
      </div>
      <h1 className="mt-6 font-display font-normal text-[clamp(2.75rem,8vw,5.5rem)] leading-[0.98] tracking-[-0.04em]">Nothing moves here.</h1>
      <p className="mt-5 max-w-xl text-lg text-muted-foreground">
        Try the{" "}
        <a className="text-foreground underline decoration-input underline-offset-[3px] hover:text-brand hover:decoration-brand" href={at.parts}>
          parts
        </a>{" "}
        or the{" "}
        <a className="text-foreground underline decoration-input underline-offset-[3px] hover:text-brand hover:decoration-brand" href={at.blocks}>
          Blocks
        </a>
        .
      </p>
      <div className="mt-10">
        <a
          className="inline-flex h-12 items-center justify-center border border-brand bg-brand px-6 font-mono font-bold text-[0.8125rem] text-brand-foreground uppercase tracking-[0.06em] transition-transform [transition-timing-function:steps(3,end)] hover:translate-y-px active:translate-y-0.5"
          href="/"
        >
          Back home
        </a>
      </div>
    </>
  );
}
