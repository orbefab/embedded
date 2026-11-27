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

const compact = (text: string) => text.toLowerCase().replace(/[^a-z0-9]/g, "");

const hyphenTokens = (text: string) =>
  text
    .split(/[^A-Za-z0-9_-]/)
    .filter((tok) => tok.includes("-") || tok.includes("_"))
    .map(compact)
    .filter((tok) => tok !== "");

const pairsOf = (words: readonly string[]) => words.slice(0, -1).map((word, index) => `${word}${words[index + 1] ?? ""}`);

/** 0 for an exact id, 1 when every word is a whole word of the id, the title or a tag, 2 for
 * the rest, or undefined when a word is in none of the fields. */
function rank(part: Searchable, words: readonly string[], joined: string): number | undefined {
  const categories = part.categories ?? [];
  const fields = [part.name, part.title, part.maker ?? "", part.number ?? "", part.description ?? "", ...categories];
  const haystack = fields.join("\n").toLowerCase();
  const titleWords = wordsOf(part.title);
  const nameWords = wordsOf(part.name);
  const normTokens = [
    ...fields.flatMap(hyphenTokens),
    compact(part.name),
    compact(part.maker ?? ""),
    compact(part.number ?? ""),
    ...categories.map(compact),
    ...categories.flatMap(wordsOf),
    ...nameWords,
    ...titleWords,
    ...pairsOf(titleWords),
    ...pairsOf(nameWords),
  ];
  const matches = words.every((word) => {
    if (haystack.includes(word)) return true;
    const c = compact(word);
    return c !== "" && normTokens.some((tok) => tok.startsWith(c));
  });
  if (!matches) return undefined;
  const id = part.name.toLowerCase();
  const cid = compact(part.name);
  const cjoined = compact(joined);
  if (id === joined || words.includes(id) || (cjoined !== "" && cid === cjoined) || words.some((word) => { const c = compact(word); return c !== "" && c === cid; })) {
    return 0;
  }
  const whole = [
    ...nameWords,
    ...titleWords,
    ...categories.flatMap(wordsOf),
    ...categories.map(compact),
    ...pairsOf(titleWords),
    ...pairsOf(nameWords),
  ];
  return words.every((word) => {
    if (whole.includes(word)) return true;
    const c = compact(word);
    return c !== "" && whole.includes(c);
  })
    ? 1
    : 2;
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
