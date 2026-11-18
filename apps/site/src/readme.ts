// A Block's README, as the page shows it: paragraphs, inline code and links. The leading title
// repeats the page's, so it is left out. The install paragraph says `orbe add`, which the site
// shows only when install lines are on.

export type ReadmeRun =
  | { readonly kind: "text"; readonly text: string }
  | { readonly kind: "code"; readonly text: string }
  | { readonly kind: "link"; readonly text: string; readonly href: string };

const INLINE = /\[([^\]]+)\]\(([^)]+)\)|`([^`]+)`/g;

function runs(text: string): readonly ReadmeRun[] {
  const out: ReadmeRun[] = [];
  let cursor = 0;
  for (const match of text.matchAll(INLINE)) {
    const index = match.index ?? 0;
    if (index > cursor) out.push({ kind: "text", text: text.slice(cursor, index) });
    if (match[3] !== undefined) out.push({ kind: "code", text: match[3] });
    else out.push({ kind: "link", text: match[1] ?? "", href: match[2] ?? "" });
    cursor = index + match[0].length;
  }
  if (cursor < text.length) out.push({ kind: "text", text: text.slice(cursor) });
  return out;
}

/** The paragraphs the page shows. `showInstall` keeps the paragraph that says `orbe add`. */
export function readmeParagraphs(markdown: string, showInstall: boolean): readonly (readonly ReadmeRun[])[] {
  return markdown
    .replace(/^# [^\n]*\n*/, "")
    .split(/\n\s*\n/)
    .map((paragraph) => paragraph.trim())
    .filter((paragraph) => paragraph !== "" && (showInstall || !paragraph.includes("orbe add")))
    .map(runs);
}
