import type { ReactNode } from "react";
import { BlockPlayer } from "../components/block-player";
import { BlockStage } from "../components/block-stage";
import { lockSentences, type BlockCircuit } from "../circuit";
import type { BlockCard, Linked } from "../data";
import { at } from "../paths";
import type { ReadmeRun } from "../readme";
import { type BlockRecord, type FileEntry, type Item } from "../records";
import { Files, LicenseBox } from "./files";
import { CliComing, Code, Install, Section } from "./layout";

export function BlockCards({ blocks }: { blocks: readonly BlockCard[] }) {
  return (
    <ul className="grid gap-4 sm:grid-cols-2">
      {blocks.map((block) => (
        <li key={block.name}>
          <a className="block h-full border border-border p-5 transition-colors hover:border-brand" href={at.block(block.name)}>
            <span className="font-medium">{block.title}</span>
            <span className="mt-2 block text-muted-foreground text-sm">{block.description ?? "No summary yet."}</span>
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
      <p className="mt-3 max-w-2xl text-muted-foreground text-lg">
        Small circuits that work, each with its scene, its firmware and its tests. Each one plays live in your browser on the real engine.
      </p>
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

/** Where a README link goes. `firmware/NOTICE` opens the license box. A web or mail address
 * stays as written. Any other target is a file of the Block, or no link. */
function fileHref(href: string, files: readonly FileEntry[]): string | undefined {
  if (href === "firmware/NOTICE" || href.endsWith("/firmware/NOTICE")) return "#files";
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
  const { board, placed } = circuit;
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
                <span className="mt-0.5 block font-mono text-muted-foreground text-xs">{part.id}</span>
              </span>
            </div>
            {part.id === board.id ? <BoardFold circuit={circuit} links={links} /> : null}
          </li>
        ))}
      </ul>
    </div>
  );
}

function CodeWords({ text, words }: { text: string; words: readonly string[] }) {
  const sorted = [...words].filter((word) => word.length > 0).sort((a, b) => b.length - a.length);
  const nodes: ReactNode[] = [];
  let rest = text;
  let key = 0;
  while (rest.length > 0) {
    let foundAt = -1;
    let found = "";
    for (const word of sorted) {
      const at = rest.indexOf(word);
      if (at < 0) continue;
      const before = at === 0 || /[^A-Za-z0-9_-]/.test(rest[at - 1] ?? "");
      const after = at + word.length === rest.length || /[^A-Za-z0-9_-]/.test(rest[at + word.length] ?? "");
      if (!before || !after) continue;
      if (foundAt < 0 || at < foundAt) {
        foundAt = at;
        found = word;
      }
    }
    if (foundAt < 0 || found === "") {
      nodes.push(rest);
      break;
    }
    if (foundAt > 0) nodes.push(rest.slice(0, foundAt));
    nodes.push(<Code key={key}>{found}</Code>);
    key += 1;
    rest = rest.slice(foundAt + found.length);
  }
  return <>{nodes}</>;
}

function BoardFold({ circuit, links }: { circuit: BlockCircuit; links: BlockView["links"] }) {
  const { board } = circuit;
  const { brings, lists } = lockSentences(circuit);
  return (
    <details className="mt-3 text-muted-foreground text-sm">
      <summary className="cursor-pointer">
        {brings}{" "}
        {__ORBE_SITE_INSTALL__ ? (
          <>
            <Code>orbe add</Code> copies them too.{" "}
          </>
        ) : null}
        <CodeWords text={lists} words={["parts.lock", ...new Set(circuit.placed.map((part) => part.id))]} />
      </summary>
      <ul className="mt-2 columns-1 gap-8 sm:columns-2">
        {board.own.map((part) => (
          <li className="break-inside-avoid py-0.5" key={part.id}>
            <a className="underline hover:text-foreground" href={links[part.id]?.href ?? at.part(part.id)}>
              {links[part.id]?.title ?? part.title}
            </a>
          </li>
        ))}
      </ul>
      <p className="mt-2">
        <a className="underline hover:text-foreground" href={at.part(board.id)}>
          See them on the board's page
        </a>
      </p>
    </details>
  );
}

function testText(tests: number): string {
  return tests === 1 ? "1 test" : `${tests} tests`;
}

function InstallSlot({ name, tests }: { name: string; tests: number }) {
  if (!__ORBE_SITE_INSTALL__) return <CliComing />;
  const checksText = testText(tests);
  return (
    <div>
      <Install lines={[`orbe add ${name}`]} />
      <details className="mt-2 text-muted-foreground text-sm">
        <summary className="cursor-pointer">
          Run it in an empty folder. <span className="underline">New to orbe?</span>
        </summary>
        <ol className="mt-2 list-decimal space-y-1 pl-5 text-foreground">
          <li>
            Get orbe from the{" "}
            <a className="underline hover:text-brand" href={at.home}>
              home page
            </a>
            .
          </li>
          <li>
            Make an empty folder and run <Code>orbe add {name}</Code> in it.
          </li>
          <li>
            Run <Code>orbe test</Code>. It runs {checksText}.
          </li>
        </ol>
        <p className="mt-2">
          The files are under Code, and under{" "}
          <a className="underline hover:text-foreground" href="#files">
            License and files
          </a>
          .
        </p>
      </details>
    </div>
  );
}

export function BlockPage({ block }: { block: BlockView }) {
  const { item, links, circuit } = block;
  const { record } = item;
  const title = (id: string) => links[id]?.title ?? id;
  const notice = record.files.find((file) => file.path === "firmware/NOTICE");
  return (
    <article className="min-w-0">
      <p className="font-mono text-muted-foreground text-xs">
        <a className="hover:text-foreground" href={at.blocks}>
          blocks
        </a>{" "}
        / {record.name}
      </p>
      <h1 className="mt-3 font-semibold text-3xl tracking-tight md:text-4xl">{record.title}</h1>
      <p className="mt-4 max-w-3xl text-lg">{record.description ?? <span className="text-muted-foreground">No summary yet.</span>}</p>
      <p className="mt-3 flex flex-wrap gap-x-4 gap-y-1 font-mono text-muted-foreground text-xs">
        <span>
          on{" "}
          <a className="underline hover:text-foreground" href={at.part(record.board)}>
            {title(record.board)}
          </a>
        </span>
        <span data-scene-parts={circuit.placed.length}>{circuit.placed.length} parts</span>
        <span>{testText(circuit.tests)}</span>
        <span>v{record.version}</span>
      </p>

      <BlockStage install={<InstallSlot name={record.name} tests={circuit.tests} />} sources={circuit.sources}>
        <BlockPlayer blocks={[{ name: record.name, title: record.title, button: block.button }]} registry={at.registry} wasm={at.wasm} />
      </BlockStage>

      <Section title="How it works">
        <Readme files={record.files} paragraphs={block.readme} />
      </Section>

      <Section title="Parts in this circuit">
        <SceneParts circuit={circuit} links={links} />
      </Section>

      <LicenseBox
        expression={record.license}
        identity={`${record.name}@${record.version}, ${record.hash}, versions ${item.versions.join(", ")}`}
        recordHref={`${at.registry}${record.name}@${record.version}.json`}
      >
        <p className="max-w-3xl text-sm">
          The scene and sources are MIT or Apache-2.0. The firmware image links the Arduino AVR core, which is LGPL-2.1-or-later.
          {notice === undefined ? null : (
            <>
              {" "}
              <a className="font-mono underline hover:text-brand" href={at.file(notice.sha256)}>
                firmware/NOTICE
              </a>{" "}
              says where that source is and how to rebuild the image.
            </>
          )}
        </p>
        <Files files={record.files} />
      </LicenseBox>
    </article>
  );
}
