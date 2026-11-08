import { BlockPlayer } from "../components/block-player";
import type { BlockCard, Linked } from "../data";
import { at } from "../paths";
import { type BlockRecord, dependencyId, type Item, type Site } from "../records";
import { Files } from "./files";
import { CliComing, Install, None, Section, Table } from "./layout";

export function BlockCards({ blocks }: { blocks: readonly BlockCard[] }) {
  return (
    <ul className="grid gap-4 sm:grid-cols-2">
      {blocks.map((block) => (
        <li key={block.name}>
          <a className="block h-full rounded-lg border border-border p-5 transition-colors hover:border-brand" href={at.block(block.name)}>
            <span className="font-medium">{block.title}</span>
            <span className="mt-2 block text-muted-foreground text-sm">{block.description ?? "No summary yet."}</span>
            <span className="mt-4 block font-mono text-muted-foreground text-xs">
              {block.board}, {block.parts} parts
            </span>
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
  readonly texts: readonly { readonly path: string; readonly text: string }[];
}

export function BlockPage({ site, block }: { site: Site; block: BlockView }) {
  const { item, links, texts } = block;
  const { record } = item;
  const title = (id: string) => links[id]?.title ?? id;
  return (
    <article>
      <p className="font-mono text-muted-foreground text-xs">
        <a className="hover:text-foreground" href={at.blocks}>
          blocks
        </a>{" "}
        / {record.name}
      </p>
      <h1 className="mt-3 font-semibold text-3xl tracking-tight md:text-4xl">{record.title}</h1>
      <p className="mt-4 max-w-3xl text-lg">{record.description ?? <span className="text-muted-foreground">No summary yet.</span>}</p>
      <p className="mt-3 font-mono text-muted-foreground text-xs">
        on{" "}
        <a className="underline hover:text-foreground" href={at.part(record.board)}>
          {title(record.board)}
        </a>
        , {record.license}
      </p>

      <div className="mt-8">
        <BlockPlayer blocks={[{ name: record.name, title: record.title }]} registry={at.registry} wasm={at.wasm} />
        <p className="mt-3 text-muted-foreground text-sm">
          This is the real engine: orbe-wasm {site.engine.version} runs the Block's firmware in a Worker in your browser. Press the button.
        </p>
      </div>

      <Section title="Install">
        {__ORBE_SITE_INSTALL__ ? (
          <Install lines={[`orbe add ${record.name}`]} note="Run it in an empty folder. It copies the parts, then writes the scene, the firmware and the tests." />
        ) : (
          <CliComing />
        )}
      </Section>

      <Section title="Parts">
        <Table
          head={["Part", "Id", "Version"]}
          rows={record.registryDependencies.map((dependency) => {
            const id = dependencyId(dependency);
            return [
              <a className="underline hover:text-brand" href={at.part(id)} key="title">
                {title(id)}
              </a>,
              <span className="font-mono text-muted-foreground" key="id">
                {id}
                {id === record.board ? ", the board" : ""}
              </span>,
              <span className="font-mono text-muted-foreground" key="version">
                {dependency.slice(id.length + 1)}
              </span>,
            ];
          })}
        />
      </Section>

      <Section title="Scene, firmware and tests">
        {texts.length === 0 ? (
          <None>The Block has no text files to show.</None>
        ) : (
          <div className="grid gap-3">
            {texts.map((file) => (
              <details className="rounded-lg border border-border" key={file.path} open={file.path === "scene.jsonc"}>
                <summary className="cursor-pointer px-4 py-2 font-mono text-sm">{file.path}</summary>
                <pre className="overflow-x-auto border-border border-t p-4 font-mono text-xs leading-relaxed">
                  {file.text}
                </pre>
              </details>
            ))}
          </div>
        )}
      </Section>

      <Section title="Licence">
        <p className="max-w-3xl">
          The Block is under <span className="font-mono">{record.license}</span>. Its scene and sources are MIT OR Apache-2.0. Its firmware images
          link the Arduino AVR core, which is LGPL-2.1-or-later, and <span className="font-mono">firmware/NOTICE</span> says where that source is
          and how to rebuild the image. Each file below lists its own licence.
        </p>
      </Section>

      <Section title="Files">
        <Files files={record.files} />
        <p className="mt-3 font-mono text-muted-foreground text-xs">
          {record.name}@{record.version}, {record.hash}, versions {item.versions.join(", ")}
        </p>
      </Section>
    </article>
  );
}
