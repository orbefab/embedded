// The catalog's scene files are JSON with comments. A string keeps its own slashes.

function copyString(text: string, start: number): number {
  let i = start + 1;
  while (i < text.length) {
    if (text[i] === "\\") {
      i += 2;
      continue;
    }
    if (text[i] === '"') return i;
    i++;
  }
  return i;
}

/** The value `text` holds, comments and trailing commas removed. */
export function parseJsonc(text: string): unknown {
  let stripped = "";
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"') {
      const end = copyString(text, i);
      stripped += text.slice(i, end + 1);
      i = end;
      continue;
    }
    if (c === "/" && text[i + 1] === "/") {
      i += 2;
      while (i < text.length && text[i] !== "\n") i++;
      continue;
    }
    if (c === "/" && text[i + 1] === "*") {
      i += 2;
      while (i < text.length && !(text[i] === "*" && text[i + 1] === "/")) i++;
      i++;
      continue;
    }
    stripped += c;
  }

  let json = "";
  for (let i = 0; i < stripped.length; i++) {
    const c = stripped[i];
    if (c === '"') {
      const end = copyString(stripped, i);
      json += stripped.slice(i, end + 1);
      i = end;
      continue;
    }
    if (c === ",") {
      let j = i + 1;
      while (j < stripped.length && /\s/.test(stripped[j] ?? "")) j++;
      if (stripped[j] === "}" || stripped[j] === "]") continue;
    }
    json += c;
  }
  return JSON.parse(json);
}
