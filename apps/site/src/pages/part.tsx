import { stripText } from "@orbe/viewer/refs";
import { PartModel } from "../components/part-model";
import type { Linked } from "../data";
import { paramSentence, si, sourceText } from "../format";
import { at } from "../paths";
import { dependencyId, type Item, type PartRecord } from "../records";
import { Files } from "./files";
import { CliComing, Code, Install, None, Section, Table } from "./layout";

export interface PartView {
  readonly item: Item<PartRecord>;
  readonly links: Readonly<Record<string, Linked>>;
}

/** The maker and number a part matches, in words. */
export function matches(record: PartRecord): string {
  const { maker, number, generic } = record.real;
  const named = [maker, number].filter(Boolean).join(" ");
  if (named === "") return "Generic part, matched to no maker's number";
  return generic ? `Generic part. ${named} is the one its values and model follow` : `Matches ${named}`;
}

/** Links to the items named, each to its own page. */
function Links({ links, names }: { links: PartView["links"]; names: readonly string[] }) {
  return (
    <ul className="grid gap-1 sm:grid-cols-2">
      {names.map((name) => (
        <li key={name}>
          <a className="underline hover:text-brand" href={links[name]?.href ?? at.part(name)}>
            {links[name]?.title ?? name}
          </a>{" "}
          <span className="font-mono text-muted-foreground text-xs">{name}</span>
        </li>
      ))}
    </ul>
  );
}

export function PartPage({ part }: { part: PartView }) {
  const { item, links } = part;
  const { record } = item;
  const children = record.registryDependencies.map(dependencyId);
  const holes = new Set(record.holes?.strips.flat());
  const pins = record.pins.filter((pin) => !holes.has(pin.name));
  return (
    <article>
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

      <div className="mt-8">
        <PartModel url={at.partView(record.name)} />
      </div>

      <Section title="Pins">
        {pins.length === 0 && record.holes ? (
          <None>Only its holes.</None>
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
          <None>It takes no params.</None>
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
              <span className="text-muted-foreground" key="source">
                {sourceText(rating.source)}
              </span>,
            ])}
          />
        )}
        {record.on === undefined ? null : (
          <p className="mt-4 text-muted-foreground text-sm">
            It reads as on while {record.on.of} is {record.on.above === undefined ? "below" : "above"}{" "}
            {si(record.on.above ?? record.on.below ?? 0, record.on.unit)}.
          </p>
        )}
      </Section>

      <Section title="Not simulated">
        {record.notSimulated.length === 0 ? (
          <None>The part file lists nothing the engine leaves out.</None>
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
          <Links links={links} names={[...new Set(children)]} />
        </Section>
      )}
      <Section title="Used by">
        {item.usedBy.length === 0 ? <None>Nothing in the catalog uses it yet.</None> : <Links links={links} names={item.usedBy} />}
      </Section>

      <Section title="Files">
        <Files files={record.files} />
        <p className="mt-3 font-mono text-muted-foreground text-xs">
          {record.name}@{record.version}, {record.hash}
        </p>
      </Section>

      <Section title="Docs">
        {record.docs === undefined ? (
          <None>No docs linked yet.</None>
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
          <None>No sellers listed yet.</None>
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

      <Section title="Install">
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
        <p className="mt-3 font-mono text-muted-foreground text-xs">versions {item.versions.join(", ")}</p>
      </Section>
    </article>
  );
}
