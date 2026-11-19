import { stripText } from "@orbe/viewer/refs";
import type { ReactNode } from "react";
import { PartModel } from "../components/part-model";
import { glanceLine, type PartUse } from "../circuit";
import type { Linked } from "../data";
import { paramSentence, si, sourceText } from "../format";
import { at } from "../paths";
import { dependencyId, type Item, type PartRecord, type Pin, type Rating } from "../records";
import { Files, LicenseBox } from "./files";
import { CliComing, Code, Install, None, Section, Table } from "./layout";

export interface PartView {
  readonly item: Item<PartRecord>;
  readonly links: Readonly<Record<string, Linked>>;
  readonly use: PartUse;
  readonly where: string;
}

const QUANTITY: Readonly<Record<string, string>> = { i: "current", v: "voltage", p: "power" };

/** The brightness ranges an LED's page states. `i` is current, not the symbol. */
function brightnessText(record: PartRecord): string | null {
  const lit = record.lit;
  const dim = record.dim;
  if (lit === undefined && dim === undefined) return null;
  const of = lit?.of ?? dim?.of ?? "";
  const name = QUANTITY[of] ?? of;
  let text = `Brightness is the average ${name} over 20 ms.`;
  if (lit !== undefined) {
    text += ` It reads lit while that average is above ${si(lit.above, lit.unit)}.`;
  }
  if (dim !== undefined) {
    const above = si(dim.above, dim.unit);
    text +=
      lit === undefined
        ? ` It reads dim while that average is above ${above}.`
        : ` It reads dim while that average is above ${above} and at or below ${si(lit.above, lit.unit)}.`;
    text += ` It reads off while that average is at or below ${above}.`;
  }
  return text;
}

/** The maker and number a part matches, in words. */
export function matches(record: PartRecord): string {
  const { maker, number, generic } = record.real;
  const named = [maker, number].filter(Boolean).join(" ");
  if (named === "") return "Generic part, matched to no maker's number";
  return generic ? `Generic part. ${named} is the one its values and model follow` : `Matches ${named}`;
}

function pinGlance(pins: readonly Pin[]): string {
  if (pins.length === 0) return "No pins yet.";
  if (pins.length > 8) return `${pins.length} pins`;
  return pins.map((pin) => pin.name).join(", ");
}

function SourceCell({ rating, docs }: { rating: Rating; docs: PartRecord["docs"] }) {
  const source = rating.source;
  const text = sourceText(source);
  if (source !== undefined && source !== "user" && "datasheet" in source) {
    const match = docs?.find((doc) => doc.name === source.datasheet.doc && doc.url !== undefined);
    if (match?.url !== undefined) {
      return (
        <span className="text-muted-foreground">
          {text}{" "}
          <a className="underline hover:text-brand" href={match.url} rel="noopener" target="_blank">
            Datasheet
          </a>
        </span>
      );
    }
  }
  return <span className="text-muted-foreground">{text}</span>;
}

function Glance({ part }: { part: PartView }) {
  const { record } = part.item;
  const holes = new Set(record.holes?.strips.flat());
  const pins = record.pins.filter((pin) => !holes.has(pin.name));
  const place = glanceLine(part.use);
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-sm">
      <dt className="font-mono text-muted-foreground text-xs">pins</dt>
      <dd>{pinGlance(pins)}</dd>
      {record.ratings.map((rating) => (
        <span className="contents" key={rating.name}>
          <dt className="font-mono text-muted-foreground text-xs">{rating.name}</dt>
          <dd className="font-mono">{rating.value === null ? "unrated" : si(rating.value, rating.unit)}</dd>
        </span>
      ))}
      {record.lit === undefined ? null : (
        <>
          <dt className="font-mono text-muted-foreground text-xs">lit</dt>
          <dd>above {si(record.lit.above, record.lit.unit)}</dd>
        </>
      )}
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
      <dt className="font-mono text-muted-foreground text-xs">version</dt>
      <dd className="break-all font-mono text-xs">{record.version}</dd>
    </dl>
  );
}

function UsedIn({ part }: { part: PartView }) {
  const { use } = part;
  const blocks =
    use.blocks.length > 0 ? (
      <Section id="used" title="Used in Blocks">
        <ul className="grid gap-3 sm:grid-cols-2" data-testid="used-in">
          {use.blocks.map((block) => (
            <li key={block.id}>
              <a className="block h-full rounded-lg border border-border p-4 transition-colors hover:border-brand" href={block.href}>
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
    ) : use.boards.length === 0 ? (
      <Section id="used" title="Used in Blocks">
        <None>In no Block yet.</None>
      </Section>
    ) : null;
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

export function PartPage({ part }: { part: PartView }) {
  const { item, links } = part;
  const { record } = item;
  const children = record.registryDependencies.map(dependencyId);
  const holes = new Set(record.holes?.strips.flat());
  const pins = record.pins.filter((pin) => !holes.has(pin.name));
  return (
    <article className="min-w-0">
      <p className="font-mono text-muted-foreground text-xs">
        <a className="hover:text-foreground" href={at.parts}>
          parts
        </a>{" "}
        / {record.name}
      </p>
      <h1 className="mt-3 font-semibold text-3xl tracking-tight md:text-4xl">{record.title}</h1>
      <p className="mt-2 text-muted-foreground">{matches(record)}.</p>
      {record.real.maker ? (
        <p className="mt-1 text-muted-foreground text-sm" data-testid="not-affiliated">
          Orbefab is not affiliated with or endorsed by {record.real.maker}. The name only says which real part this one matches.
        </p>
      ) : null}
      <p className="mt-6 max-w-3xl text-lg">{record.description ?? <span className="text-muted-foreground">No summary yet.</span>}</p>
      {record.categories === undefined ? null : (
        <p className="mt-3 flex flex-wrap gap-2 font-mono text-xs">
          {record.categories.map((tag) => (
            <a className="rounded border border-border px-2 py-0.5 hover:border-foreground" href={`${at.parts}?q=${encodeURIComponent(tag)}`} key={tag}>
              {tag}
            </a>
          ))}
        </p>
      )}

      <div className="mt-8 grid grid-cols-[minmax(0,1fr)] items-start gap-8 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <PartModel url={at.partView(record.name)} />
        <div className="grid min-w-0 grid-cols-[minmax(0,1fr)] gap-8">
          <div>
            <Label>Install</Label>
            <div className="mt-3">
              {__ORBE_SITE_INSTALL__ ? (
                <Install
                  lines={[`orbe add ${record.name}`]}
                  note={
                    <>
                      It copies the part into the project's <Code>parts/</Code> and records it in <Code>parts.lock</Code>.
                    </>
                  }
                />
              ) : (
                <CliComing />
              )}
            </div>
          </div>
          <div>
            <Label>At a glance</Label>
            <div className="mt-3">
              <Glance part={part} />
            </div>
          </div>
        </div>
      </div>

      <UsedIn part={part} />

      <Section title="Pins">
        {pins.length === 0 ? (
          <None>No pins yet.</None>
        ) : (
          <Table
            head={["Pin", pins.some((pin) => pin.refs) ? "Reaches" : "Leads"]}
            rows={pins.map((pin) => [
              <span className="font-mono" key="name">
                {pin.name}
              </span>,
              <span className="font-mono text-muted-foreground" key="at">
                {(pin.leads ?? pin.refs ?? []).join(", ")}
              </span>,
            ])}
          />
        )}
      </Section>

      {record.holes === undefined ? null : (
        <Section title="Holes">
          <p className="mb-3 text-muted-foreground text-sm">
            Each hole is a pin with one lead, {record.holes.depth} mm deep. A strip joins its holes, so a leg in one hole joins every
            other leg and jumper in its strip.
          </p>
          <Table
            head={["Strip", "Holes"]}
            rows={record.holes.strips.map((strip) => [
              <span className="font-mono" key="strip">
                {stripText(strip)}
              </span>,
              <span className="font-mono text-muted-foreground" key="holes">
                {strip.join(", ")}
              </span>,
            ])}
          />
        </Section>
      )}

      <Section title="Params">
        {record.params.length === 0 ? (
          <None>No params yet.</None>
        ) : (
          <Table
            head={["Param", "Takes"]}
            rows={record.params.map((param) => [
              <span className="font-mono" key="name">
                {param.name}
              </span>,
              <span key="takes">
                {paramSentence(param)}
                {param.choices?.map((choice) => (
                  <span className="mt-1 flex items-center gap-2 font-mono text-muted-foreground text-xs" key={choice.name}>
                    {choice.tint === undefined ? null : (
                      <span className="inline-block size-3 rounded-sm border border-border" style={{ background: choice.tint }} />
                    )}
                    {choice.name}
                    {choice.number === undefined ? null : `, ${choice.number}`}
                  </span>
                ))}
              </span>,
            ])}
          />
        )}
      </Section>

      <Section title="Ratings">
        {record.ratings.length === 0 ? (
          <None>No ratings yet.</None>
        ) : (
          <Table
            head={["Rating", "Value", "Source"]}
            rows={record.ratings.map((rating) => [
              <span className="font-mono" key="name">
                {rating.name}
              </span>,
              <span className="whitespace-nowrap font-mono" key="value">
                {rating.value === null ? "unrated" : si(rating.value, rating.unit)}
              </span>,
              <SourceCell docs={record.docs} key="source" rating={rating} />,
            ])}
          />
        )}
        {record.lit === undefined && record.dim === undefined ? null : <p className="mt-4 max-w-3xl text-muted-foreground text-sm">{brightnessText(record)}</p>}
      </Section>

      <Section title="Not simulated">
        {record.notSimulated.length === 0 ? (
          <None>Nothing left out yet.</None>
        ) : (
          <ul className="grid gap-3">
            {record.notSimulated.map((entry) => (
              <li key={entry.what}>
                <span className="font-medium">{entry.what}</span>
                <span className="block text-muted-foreground">{entry.why}</span>
              </li>
            ))}
          </ul>
        )}
      </Section>

      {children.length === 0 ? null : (
        <Section title="Built from">
          <ul className="grid gap-1 sm:grid-cols-2">
            {[...new Set(children)].map((name) => (
              <li key={name}>
                <a className="underline hover:text-brand" href={links[name]?.href ?? at.part(name)}>
                  {links[name]?.title ?? name}
                </a>{" "}
                <span className="font-mono text-muted-foreground text-xs">{name}</span>
              </li>
            ))}
          </ul>
        </Section>
      )}

      <Section title="Docs">
        {record.docs === undefined ? (
          <None>No docs yet.</None>
        ) : (
          <ul className="grid gap-1">
            {record.docs.map((doc) => (
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
        )}
      </Section>

      <Section title="Where to buy">
        {record.buy === undefined ? (
          <None>No sellers yet.</None>
        ) : (
          <>
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
            <p className="mt-3 text-muted-foreground text-sm">Each link was checked on the date beside it. These are not affiliate links.</p>
          </>
        )}
      </Section>

      <LicenseBox
        expression={record.license}
        identity={`${record.name}@${record.version}, ${record.hash}, versions ${item.versions.join(", ")}`}
        recordHref={`${at.registry}${record.name}@${record.version}.json`}
      >
        <Files files={record.files} />
      </LicenseBox>
    </article>
  );
}
