import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { compareVersions } from "@orbe/viewer/registry";
import { describe, expect, it } from "vitest";
import type { IndexRow } from "./records";
import { groupParts, PART_SECTION_BY_ID, PART_SECTIONS, sectionOfPart, validatePartSections } from "./sections";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

function parseJsonc(text: string): unknown {
  let out = "";
  let inString = false;
  let escaped = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    const next = text[i + 1];
    if (inString) {
      out += ch;
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === "\"") inString = false;
      continue;
    }
    if (ch === "\"") {
      inString = true;
      out += ch;
      continue;
    }
    if (ch === "/" && next === "/") {
      while (i < text.length && text[i] !== "\n") i++;
      out += "\n";
      continue;
    }
    if (ch === "/" && next === "*") {
      i += 2;
      while (i < text.length && !(text[i] === "*" && text[i + 1] === "/")) i++;
      i++;
      continue;
    }
    out += ch;
  }
  return JSON.parse(out.replace(/,(\s*[}\]])/g, "$1"));
}

function repoParts(): { existing: string[]; listed: string[] } {
  const partsDir = join(REPO_ROOT, "parts");
  const existing = readdirSync(partsDir)
    .filter((entry) => existsSync(join(partsDir, entry, "part.jsonc")))
    .sort();
  const manifest = parseJsonc(readFileSync(join(REPO_ROOT, "registry.jsonc"), "utf8")) as {
    unlisted?: readonly string[];
  };
  const unlisted = new Set(manifest.unlisted ?? []);
  const listed = new Set(existing.filter((id) => !unlisted.has(id)));
  // When a built registry is present, include every part whose newest published index row is listed.
  const indexPath = join(REPO_ROOT, ".scratch/site/registry/index.json");
  if (existsSync(indexPath)) {
    const index = JSON.parse(readFileSync(indexPath, "utf8")) as { items: readonly IndexRow[] };
    const newest = new Map<string, IndexRow>();
    for (const row of index.items) {
      if (row.type !== "part") continue;
      const prev = newest.get(row.name);
      if (prev === undefined || compareVersions(prev.version, row.version) < 0) newest.set(row.name, row);
    }
    for (const row of newest.values()) {
      if (row.listed !== false) listed.add(row.name);
    }
  }
  return { existing, listed: [...listed].sort() };
}

describe("part sections", () => {
  it("maps every listed repo part and names no missing part", () => {
    const { existing, listed } = repoParts();
    expect(listed.length).toBeGreaterThan(0);
    expect(() => validatePartSections(listed, existing, PART_SECTION_BY_ID)).not.toThrow();
  });

  it("fails with the part name when a listed part has no section", () => {
    expect(() =>
      validatePartSections(["led-5mm", "new-unmapped-sensor"], ["led-5mm", "new-unmapped-sensor"], {
        "led-5mm": "LEDs and lights",
      }),
    ).toThrow(/new-unmapped-sensor/);
  });

  it("fails with the part name when the table names a part that does not exist", () => {
    expect(() =>
      validatePartSections(["led-5mm"], ["led-5mm"], {
        "led-5mm": "LEDs and lights",
        "ghost-part-99": "Sensors",
      }),
    ).toThrow(/ghost-part-99/);
  });

  it("groups parts in fixed section order and omits empty sections", () => {
    const groups = groupParts(
      [
        { name: "usb-b-socket" },
        { name: "led-5mm" },
        { name: "nano-board" },
        { name: "led-3mm" },
      ],
      PART_SECTION_BY_ID,
    );
    expect(groups.map((group) => group.section)).toEqual([
      "Boards and microcontrollers",
      "LEDs and lights",
      "Connectors and sockets",
    ]);
    expect(groups[1]?.parts.map((part) => part.name)).toEqual(["led-5mm", "led-3mm"]);
  });

  it("throws naming the part when looking up an unmapped part", () => {
    expect(() => sectionOfPart("unknown-widget")).toThrow(/unknown-widget/);
  });

  it("has non-empty sections covering every entry in PART_SECTIONS", () => {
    const { listed } = repoParts();
    const groups = groupParts(listed.map((name) => ({ name })));
    expect(groups.map((group) => group.section)).toEqual([...PART_SECTIONS]);
    expect(groups.reduce((sum, group) => sum + group.parts.length, 0)).toBe(listed.length);
  });
});
