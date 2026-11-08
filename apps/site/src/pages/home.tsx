import { BlockPlayer } from "../components/block-player";
import type { BlockCard, Linked } from "../data";
import { at } from "../paths";
import type { Site } from "../records";
import { BlockCards } from "./blocks";
import { CliComing, Install, Section } from "./layout";

export interface Home {
  readonly blocks: readonly BlockCard[];
  readonly boards: readonly Linked[];
  readonly parts: number;
}

export function HomePage({ site, home }: { site: Site; home: Home }) {
  return (
    <>
      <section className="grid gap-10 lg:grid-cols-[minmax(0,0.85fr)_minmax(0,1.15fr)] lg:items-center">
        <div>
          <h1 className="font-mono font-semibold text-5xl tracking-tight">orbe</h1>
          <p className="mt-5 text-2xl leading-snug">A hardware simulator for AI agents.</p>
          <p className="mt-5 max-w-xl text-muted-foreground">
            You describe a circuit as a scene file, flash it with real firmware, and drive it from the command line with orbe. It runs Arduino Nano and
            Uno compatible boards headless for an agent, or live in 3D in the browser, as here.
          </p>
          <p className="mt-6 flex gap-5 font-mono text-xs uppercase tracking-[0.16em]">
            <a className="text-brand hover:underline" href={at.parts}>
              {home.parts} parts
            </a>
            <a className="text-brand hover:underline" href={at.blocks}>
              {home.blocks.length} Blocks
            </a>
          </p>
        </div>
        <div>
          <BlockPlayer blocks={home.blocks.map(({ name, title }) => ({ name, title }))} registry={at.registry} wasm={at.wasm} />
          <p className="mt-3 text-muted-foreground text-sm">
            The real engine, orbe-wasm {site.engine.version}, runs each Block's firmware in your browser. Click the button to press it.
          </p>
        </div>
      </section>

      <Section title="Get orbe">
        {__ORBE_SITE_INSTALL__ ? (
          <Install
            lines={["cargo install --locked --git https://github.com/orbefab/embedded orbe", "orbe skills get core"]}
            note={
              <>
                Binaries for Linux and macOS are on the{" "}
                <a className="underline hover:text-foreground" href="https://github.com/orbefab/embedded/releases">
                  releases page
                </a>
                . <code className="font-mono">orbe skills get core</code> prints how an agent uses orbe.
              </>
            }
          />
        ) : (
          <CliComing />
        )}
      </Section>

      <Section title="Blocks">
        <BlockCards blocks={home.blocks} />
      </Section>

      <Section title="Boards">
        <ul className="grid gap-2">
          {home.boards.map((board) => (
            <li key={board.href}>
              <a className="underline hover:text-brand" href={board.href}>
                {board.title}
              </a>
            </li>
          ))}
        </ul>
        <p className="mt-4">
          <a className="text-brand hover:underline" href={at.parts}>
            Every part in the catalog
          </a>
        </p>
      </Section>
    </>
  );
}

export function NotFoundPage() {
  return (
    <>
      <h1 className="font-semibold text-3xl tracking-tight">Not here</h1>
      <p className="mt-4 text-muted-foreground">
        Nothing is at this address. Try the{" "}
        <a className="underline hover:text-foreground" href={at.parts}>
          parts
        </a>{" "}
        or the{" "}
        <a className="underline hover:text-foreground" href={at.blocks}>
          Blocks
        </a>
        .
      </p>
    </>
  );
}
