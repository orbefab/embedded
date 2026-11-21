import type { ReactNode } from "react";
import { BlockPlayer } from "../components/block-player";
import { BlockStage } from "../components/block-stage";
import type { BlockCircuit } from "../circuit";
import type { BlockCard, Linked } from "../data";
import { at } from "../paths";
import type { ReadmeRun } from "../readme";
import { type BlockRecord, type FileEntry, type Item } from "../records";
import { Files, LicenseBox } from "./files";
import { CliComing, Install, Section } from "./layout";

export function BlockCards({ blocks }: { blocks: readonly BlockCard[] }) {
  return (
    <ul className="grid gap-4 sm:grid-cols-2">
      {blocks.map((block) => (
        <li key={block.name}>
          <a className="block h-full border border-border p-5 transition-colors hover:border-brand" href={at.block(block.name)}>
            <span className="font-medium">{block.title}</span>
            {block.description === undefined ? null : <span className="mt-2 block text-muted-foreground text-sm">{block.description}</span>}
            <span className="mt-4 block font-mono text-muted-foreground text-xs" data-scene-parts={block.parts}>{`${block.board}, ${block.parts} parts`}</span>
          </a>
        </li>
      ))}
    </ul>
  );
}

export function BlocksPage({ blocks }: { blocks: readonly BlockCard[] }) {
  return (
    <>
      <h1 className="font-semibold text-3xl tracking-tight md:text-4xl">Blocks</h1>
      <p className="mt-3 max-w-2xl text-muted-foreground text-lg">Small circuits that work. Play each one live in your browser.</p>
      <div className="mt-10">
        <BlockCards blocks={blocks} />
      </div>
    </>
  );
}

export interface BlockView {
  readonly item: Item<BlockRecord>;
  readonly links: Readonly<Record<string, Linked>>;
  readonly circuit: BlockCircuit;
  readonly readme: readonly (readonly ReadmeRun[])[];
  /** The scene places a part with a press input, so the page shows the hold control. */
  readonly button: boolean;
}

/** Where a README link goes. A web or mail address stays as written. Any other target is a
 * file of the Block, or no link. */
function fileHref(href: string, files: readonly FileEntry[]): string | undefined {
  if (/^(https?:\/\/|mailto:)/i.test(href)) return href;
  const file = files.find((entry) => entry.path === href);
  return file === undefined ? undefined : at.file(file.sha256);
}

function Readme({ paragraphs, files }: { paragraphs: BlockView["readme"]; files: readonly FileEntry[] }) {
  return (
    <div className="grid max-w-3xl gap-4 leading-relaxed" data-testid="readme">
      {paragraphs.map((paragraph) => (
        <p key={paragraph.map((run) => run.text).join("")}>
          {paragraph.map((run, index): ReactNode => {
            if (run.kind === "code")
              return (
                <code className="font-mono" key={index}>
                  {run.text}
                </code>
              );
            if (run.kind === "link") {
              const coded = run.text.startsWith("`") && run.text.endsWith("`");
              const text = coded ? run.text.slice(1, -1) : run.text;
              const href = fileHref(run.href, files);
              if (href === undefined)
                return coded ? (
                  <code className="font-mono" key={index}>
                    {text}
                  </code>
                ) : (
                  text
                );
              return (
                <a className="underline hover:text-brand" href={href} key={index}>
                  {coded ? <code className="font-mono">{text}</code> : text}
                </a>
              );
            }
            return run.text;
          })}
        </p>
      ))}
    </div>
  );
}

function SceneParts({ circuit, links }: { circuit: BlockCircuit; links: BlockView["links"] }) {
  const { placed } = circuit;
  const title = (id: string) => links[id]?.title ?? id;
  return (
    <div>
      <ul className="border-border border-t" data-scene-parts={placed.length} data-testid="scene-parts">
        {placed.map((part) => (
          <li className="border-border border-b py-3" data-ref={part.ref} key={part.ref}>
            <div className="grid grid-cols-[4.5rem_minmax(0,1fr)] items-baseline gap-x-4">
              <span className="font-mono text-brand text-sm">{part.ref}</span>
              <span className="min-w-0">
                <a className="underline hover:text-brand" href={at.part(part.id)}>
                  {title(part.id)}
                </a>
                {part.values.length === 0 ? null : (
                  <span className="ml-2 break-words font-mono text-xs">{part.values.join(", ")}</span>
                )}
              </span>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

function testText(tests: number): string {
  return tests === 1 ? "1 test" : `${tests} tests`;
}

function InstallSlot({ name }: { name: string }) {
  if (!__ORBE_SITE_INSTALL__) return <CliComing />;
  return <Install lines={[`orbe add ${name}`]} />;
}

export function BlockPage({ block }: { block: BlockView }) {
  const { item, links, circuit } = block;
  const { record } = item;
  const title = (id: string) => links[id]?.title ?? id;
  return (
    <article className="min-w-0">
      <p className="font-mono text-muted-foreground text-xs">
        <a className="hover:text-foreground" href={at.blocks}>
          blocks
        </a>{" "}
        / {record.name}
      </p>
      <h1 className="mt-3 font-semibold text-3xl tracking-tight md:text-4xl">{record.title}</h1>
      {record.description === undefined ? null : <p className="mt-4 max-w-3xl text-lg">{record.description}</p>}
      <p className="mt-3 flex flex-wrap gap-x-4 gap-y-1 font-mono text-muted-foreground text-xs">
        <span>
          on{" "}
          <a className="underline hover:text-foreground" href={at.part(record.board)}>
            {title(record.board)}
          </a>
        </span>
        <span data-scene-parts={circuit.placed.length}>{circuit.placed.length} parts</span>
        <span>{testText(circuit.tests)}</span>
      </p>

      <BlockStage install={<InstallSlot name={record.name} />} sources={circuit.sources}>
        <BlockPlayer blocks={[{ name: record.name, title: record.title, button: block.button }]} registry={at.registry} wasm={at.wasm} />
      </BlockStage>

      <Section title="How it works">
        <Readme files={record.files} paragraphs={block.readme} />
      </Section>

      <Section title="Parts in this circuit">
        <SceneParts circuit={circuit} links={links} />
      </Section>

      <LicenseBox expression={record.license}>
        <Files expression={record.license} files={record.files} />
      </LicenseBox>
    </article>
  );
}
