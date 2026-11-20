import type { ReactNode } from "react";
import { PartModel } from "../components/part-model";
import { glanceLine, type PartUse } from "../circuit";
import type { Linked } from "../data";
import { paramSentence, ratingName, si } from "../format";
import { at } from "../paths";
import { dependencyId, type Item, type PartRecord, type Pin, type Rating } from "../records";
import { Files, LicenseBox } from "./files";
import { CliComing, Install, Section, Table } from "./layout";

export interface PartView {
  readonly item: Item<PartRecord>;
  readonly links: Readonly<Record<string, Linked>>;
  readonly use: PartUse;
  readonly where: string;
  /** The value the 3D view is drawn at, when the part file names one the view shows. */
  readonly shown?: string;
}

function pinNames(pins: readonly Pin[]): string {
  if (pins.length > 8) return `${pins.length} pins`;
  return pins.map((pin) => pin.name).join(", ");
}

/** The datasheet link a rating cites, when its doc has a URL. */
function sourceUrl(rating: Rating, docs: PartRecord["docs"]): string | undefined {
  const source = rating.source;
  if (source === undefined || source === "user" || !("datasheet" in source)) return undefined;
  return docs?.find((doc) => doc.name === source.datasheet.doc && doc.url !== undefined)?.url;
}

function datasheetDocs(record: PartRecord): Set<string> {
  const names = new Set<string>();
  for (const rating of record.ratings) {
    const source = rating.source;
    if (source === undefined || source === "user" || !("datasheet" in source)) continue;
    const name = source.datasheet.doc;
    if (record.docs?.some((doc) => doc.name === name && doc.url !== undefined)) names.add(name);
  }
  return names;
}

function Glance({ part }: { part: PartView }) {
  const { record } = part.item;
  const holes = new Set(record.holes?.strips.flat());
  const pins = record.pins.filter((pin) => !holes.has(pin.name));
  const place = glanceLine(part.use);
  if (pins.length === 0 && place === undefined) return null;
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-sm">
      {pins.length === 0 ? null : (
        <>
          <dt className="font-mono text-muted-foreground text-xs">pins</dt>
          <dd>{pinNames(pins)}</dd>
        </>
      )}
      {place === undefined ? null : (
        <>
          <dt className="font-mono text-muted-foreground text-xs">{place.label}</dt>
          <dd>
            {place.href === undefined ? (
              place.value
            ) : (
              <a className="underline hover:text-brand" href={place.href}>
                {place.value}
              </a>
            )}
          </dd>
        </>
      )}
    </dl>
  );
}

function UsedIn({ part }: { part: PartView }) {
  const { use } = part;
  const blocks =
    use.blocks.length === 0 ? null : (
      <Section id="used" title="Used in Blocks">
        <ul className="grid gap-3 sm:grid-cols-2" data-testid="used-in">
          {use.blocks.map((block) => (
            <li key={block.id}>
              <a className="block h-full border border-border p-4 transition-colors hover:border-brand" href={block.href}>
                <span className="font-medium">{block.title}</span>
                {block.refs.map((ref) => (
                  <span className="mt-1 block text-muted-foreground text-sm" key={ref.ref}>
                    <span className="font-mono text-brand">{ref.ref}</span>
                    {ref.values.length === 0 ? null : `, ${ref.values.join(", ")}`}
                  </span>
                ))}
              </a>
            </li>
          ))}
        </ul>
      </Section>
    );
  const boards =
    use.boards.length > 0 ? (
      <Section id="boards" title="On boards">
        <ul className="grid gap-1" data-testid="on-boards">
          {use.boards.map((board) => (
            <li key={board.id}>
              On the{" "}
              <a className="underline hover:text-brand" href={board.href}>
                {board.title}
              </a>
            </li>
          ))}
        </ul>
      </Section>
    ) : null;
  return (
    <>
      {blocks}
      {boards}
    </>
  );
}

function Label({ children }: { children: ReactNode }) {
  return <h2 className="font-mono text-muted-foreground text-xs uppercase tracking-[0.16em]">{children}</h2>;
}

function Pins({ pins }: { pins: readonly Pin[] }) {
  if (pins.length === 0) return null;
  const reaches = pins.some((pin) => (pin.refs?.length ?? 0) > 0);
  if (reaches) {
    return (
      <Section title="Pins">
        <ul className="grid grid-cols-4 gap-x-6 gap-y-1 font-mono text-sm sm:grid-cols-8">
          {pins.map((pin) => (
            <li key={pin.name}>{pin.name}</li>
          ))}
        </ul>
      </Section>
    );
  }
  return (
    <Section title="Pins">
      <Table
        head={["Pin", "Leads"]}
        rows={pins.map((pin) => [
          <span className="font-mono" key="name">
            {pin.name}
          </span>,
          <span className="font-mono text-muted-foreground" key="at">
            {(pin.leads ?? []).join(", ")}
          </span>,
        ])}
      />
    </Section>
  );
}

export function PartPage({ part }: { part: PartView }) {
  const { item, links } = part;
  const { record } = item;
  const children = record.registryDependencies.map(dependencyId);
  const holes = new Set(record.holes?.strips.flat());
  const pins = record.pins.filter((pin) => !holes.has(pin.name));
  const showGlance = pins.length > 0 || glanceLine(part.use) !== undefined;
  const linkedDocs = datasheetDocs(record);
  const sourced = record.ratings.some((rating) => sourceUrl(rating, record.docs) !== undefined);
  const docs = record.docs?.filter((doc) => !linkedDocs.has(doc.name));
  return (
    <article className="min-w-0">
      <p className="font-mono text-muted-foreground text-xs">
        <a className="hover:text-foreground" href={at.parts}>
          parts
        </a>{" "}
        / {record.name}
      </p>
      <h1 className="mt-3 font-semibold text-3xl tracking-tight md:text-4xl">{record.title}</h1>
      {record.description === undefined ? null : <p className="mt-6 max-w-3xl text-lg">{record.description}</p>}
      {record.categories === undefined ? null : (
        <p className="mt-3 flex flex-wrap gap-2 font-mono text-xs">
          {record.categories.map((tag) => (
            <a className="border border-border px-2 py-0.5 hover:border-foreground" href={`${at.parts}?q=${encodeURIComponent(tag)}`} key={tag}>
              {tag}
            </a>
          ))}
        </p>
      )}

      <div className="mt-8 grid grid-cols-[minmax(0,1fr)] items-start gap-8 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <div className="min-w-0">
          <PartModel shown={part.shown} url={at.partView(record.name)} />
        </div>
        <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-8">
          <div>
            <Label>Install</Label>
            <div className="mt-3">
              {__ORBE_SITE_INSTALL__ ? <Install lines={[`orbe add ${record.name}`]} /> : <CliComing />}
            </div>
          </div>
          {showGlance ? (
            <div>
              <Label>At a glance</Label>
              <div className="mt-3">
                <Glance part={part} />
              </div>
            </div>
          ) : null}
        </div>
      </div>

      <UsedIn part={part} />

      <Pins pins={pins} />

      {record.holes === undefined ? null : (
        <Section title="Holes">
          <p className="text-muted-foreground text-sm">Holes in a strip are connected.</p>
        </Section>
      )}

      {record.params.length === 0 ? null : (
        <Section title="Settings">
          <Table
            head={["Setting", "Takes"]}
            rows={record.params.map((param) => [
              <span className="font-mono" key="name">
                {param.name}
              </span>,
              <span key="takes">
                {paramSentence(param)}
                {param.choices?.map((choice) => (
                  <span className="mt-1 flex items-center gap-2 font-mono text-muted-foreground text-xs" key={choice.name}>
                    {choice.tint === undefined ? null : (
                      <span className="inline-block size-3 border border-border" style={{ background: choice.tint }} />
                    )}
                    {choice.name}
                    {choice.number === undefined ? null : `, ${choice.number}`}
                  </span>
                ))}
              </span>,
            ])}
          />
        </Section>
      )}

      {record.ratings.length === 0 ? null : (
        <Section title="Ratings">
          <Table
            head={sourced ? ["Rating", "Value", "Source"] : ["Rating", "Value"]}
            rows={record.ratings.map((rating) => {
              const url = sourceUrl(rating, record.docs);
              const cells = [
                <span key="name">{ratingName(rating.name)}</span>,
                <span className="whitespace-nowrap font-mono" key="value">
                  {rating.value === null ? "unrated" : si(rating.value, rating.unit)}
                </span>,
              ];
              if (!sourced) return cells;
              return [
                ...cells,
                url === undefined ? null : (
                  <a className="underline hover:text-brand" href={url} key="source" rel="noopener" target="_blank">
                    Datasheet
                  </a>
                ),
              ];
            })}
          />
        </Section>
      )}

      {record.notSimulated.length === 0 ? null : (
        <Section title="Not simulated">
          <ul className="grid gap-3">
            {record.notSimulated.map((entry) => (
              <li key={entry.what}>
                <details>
                  <summary className="cursor-pointer font-medium">{entry.what}</summary>
                  <p className="mt-1 text-muted-foreground">{entry.why}</p>
                </details>
              </li>
            ))}
          </ul>
        </Section>
      )}

      {children.length === 0 ? null : (
        <Section title="Built from">
          <ul className="grid gap-1 sm:grid-cols-2">
            {[...new Set(children)].map((name) => (
              <li key={name}>
                <a className="underline hover:text-brand" href={links[name]?.href ?? at.part(name)}>
                  {links[name]?.title ?? name}
                </a>
              </li>
            ))}
          </ul>
        </Section>
      )}

      {docs === undefined || docs.length === 0 ? null : (
        <Section title="Docs">
          <ul className="grid gap-1">
            {docs.map((doc) => (
              <li key={doc.name}>
                {doc.url === undefined ? (
                  doc.name
                ) : (
                  <a className="underline hover:text-brand" href={doc.url} rel="noopener" target="_blank">
                    {doc.name}
                  </a>
                )}
              </li>
            ))}
          </ul>
        </Section>
      )}

      {record.buy === undefined ? null : (
        <Section title="Where to buy">
          <Table
            head={["Seller", "Checked"]}
            rows={record.buy.map((entry) => [
              <a className="underline hover:text-brand" href={entry.url} key="seller" rel="noopener" target="_blank">
                {entry.seller}
              </a>,
              <span className="font-mono text-muted-foreground" key="checked">
                {entry.checked}
              </span>,
            ])}
          />
        </Section>
      )}

      <LicenseBox expression={record.license}>
        <Files expression={record.license} files={record.files} />
      </LicenseBox>
    </article>
  );
}
