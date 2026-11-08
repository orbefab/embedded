// `orbe search`'s rule, from `search` in packages/scene/src/summary.rs. scripts/check-site.sh
// compares the two on the same queries.

/** The fields `orbe search` reads of a part. */
export interface Searchable {
  readonly name: string;
  readonly title: string;
  readonly maker?: string | null;
  readonly number?: string | null;
  readonly description?: string;
  readonly categories?: readonly string[];
}

const wordsOf = (text: string) =>
  text
    .split(/[^A-Za-z0-9]/)
    .filter((word) => word !== "")
    .map((word) => word.toLowerCase());

/** 0 for an exact id, 1 when every word is a whole word of the id, the title or a tag, 2 for
 * the rest, or undefined when a word is in none of the fields. */
function rank(part: Searchable, words: readonly string[], joined: string): number | undefined {
  const haystack = [part.name, part.title, part.maker ?? "", part.number ?? "", part.description ?? "", ...(part.categories ?? [])]
    .join("\n")
    .toLowerCase();
  if (!words.every((word) => haystack.includes(word))) return undefined;
  const id = part.name.toLowerCase();
  if (id === joined || words.includes(id)) return 0;
  const whole = [part.name, part.title, ...(part.categories ?? [])].flatMap(wordsOf);
  return words.every((word) => whole.includes(word)) ? 1 : 2;
}

/** The parts whose id, title, maker, number, summary or tags hold every word of `query`, best
 * first, ids breaking ties. No words find nothing, as `orbe search` does. */
export function search<P extends Searchable>(parts: readonly P[], query: string): P[] {
  const words = query
    .split(/\s+/)
    .filter((word) => word !== "")
    .map((word) => word.toLowerCase());
  if (words.length === 0) return [];
  const joined = words.join(" ");
  return parts
    .flatMap((part) => {
      const at = rank(part, words, joined);
      return at === undefined ? [] : [{ at, part }];
    })
    .sort((a, b) => a.at - b.at || (a.part.name < b.part.name ? -1 : a.part.name > b.part.name ? 1 : 0))
    .map(({ part }) => part);
}
