import { describe, expect, it } from "vitest";
import { glanceLine, lockSentences, readCatalogUse, whereLine, type BlockCircuit, type PartUse } from "./circuit";
import type { BlockRecord, FileEntry, Item, PartRecord } from "./records";

function file(path: string, sha256: string): FileEntry {
  return { path, url: path, sha256, bytes: 4, license: "MIT OR Apache-2.0" };
}

function partItem(name: string, record: Partial<PartRecord> = {}): Item<PartRecord> {
  return {
    record: {
      format: 1,
      type: "part",
      version: "0.1.0",
      title: name,
      license: "MIT",
      registryDependencies: [],
      files: [],
      hash: "h",
      real: {},
      pins: [],
      params: [],
      ratings: [],
      notSimulated: [],
      ...record,
      name,
    },
    versions: ["0.1.0"],
    usedBy: [],
  };
}

function blockItem(name: string, board: string, scene: string, extra: readonly FileEntry[] = [], deps: readonly string[] = []): { item: Item<BlockRecord>; texts: Record<string, string> } {
  const sceneFile = file("scene.jsonc", `scene-${name}`);
  const texts: Record<string, string> = { [sceneFile.sha256]: scene };
  for (const entry of extra) texts[entry.sha256] = entry.path.endsWith(".test") ? "expect one\n  expect two\n" : entry.path;
  return {
    texts,
    item: {
      record: {
        format: 1,
        type: "block",
        name,
        version: "0.1.0",
        title: name,
        license: "MIT",
        board,
        registryDependencies: [...deps],
        files: [sceneFile, ...extra],
        hash: "h",
      },
      versions: ["0.1.0"],
      usedBy: [],
    },
  };
}

function useOf(name: string, board: string, scene: string, parts: readonly Item<PartRecord>[], extra: readonly FileEntry[] = [], deps: readonly string[] = []) {
  const { item, texts } = blockItem(name, board, scene, extra, deps);
  return readCatalogUse([item], parts, texts);
}

const placed = (ref: string, id: string): BlockCircuit["placed"][number] => ({ ref, id, values: [] });

describe("readCatalogUse", () => {
  it("keeps a part that a scene places and a board carries on both lists", () => {
    const led = partItem("led", { title: "LED" });
    const board = partItem("nano-board", { title: "Nano", categories: ["board"], registryDependencies: ["led@0.1.0"] });
    const { uses } = useOf(
      "demo",
      "nano-board",
      JSON.stringify({ parts: { nano: { type: "nano-board" }, led1: { type: "led" } } }),
      [led, board],
      [],
      ["nano-board@0.1.0", "led@0.1.0"],
    );
    expect(uses.led?.blocks.map((block) => block.id)).toEqual(["demo"]);
    expect(uses.led?.boards.map((entry) => entry.id)).toEqual(["nano-board"]);
    expect(uses["nano-board"]?.blocks).toHaveLength(1);
    expect(uses["nano-board"]?.boards).toEqual([]);
  });

  it("leaves a board-only part off the Block list and a scene-only part off the board list", () => {
    const cap = partItem("cap");
    const resistor = partItem("resistor");
    const board = partItem("nano-board", { categories: ["board"], registryDependencies: ["cap@0.1.0"] });
    const { uses } = useOf(
      "demo",
      "nano-board",
      JSON.stringify({ parts: { nano: { type: "nano-board" }, r1: { type: "resistor" } } }),
      [cap, resistor, board],
    );
    expect(uses.cap?.blocks).toEqual([]);
    expect(uses.cap?.boards.map((entry) => entry.title)).toEqual(["nano-board"]);
    expect(uses.resistor?.blocks).toHaveLength(1);
    expect(uses.resistor?.boards).toEqual([]);
  });

  it("fails the build when the scene does not place its board", () => {
    expect(() => useOf("blink", "nano-board", JSON.stringify({ parts: { r1: { type: "resistor" } } }), [])).toThrow(
      "blink scene.jsonc does not place its board nano-board",
    );
  });

  it("names the file when the scene does not parse", () => {
    let caught: unknown;
    try {
      useOf("blink", "nano-board", "{", []);
    } catch (error) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(Error);
    expect(caught).not.toBeInstanceOf(SyntaxError);
    expect((caught as Error).message).toMatch(/^blink scene\.jsonc: /);
  });

  it("names the file when parts or a part is the wrong shape", () => {
    expect(() => useOf("blink", "nano-board", JSON.stringify({ parts: [] }), [])).toThrow("blink scene.jsonc parts is not an object");
    expect(() => useOf("blink", "nano-board", JSON.stringify({ parts: { a: 1 } }), [])).toThrow("blink scene.jsonc part a is not an object");
    expect(() => useOf("blink", "nano-board", JSON.stringify({ parts: { a: {} } }), [])).toThrow("blink scene.jsonc part a has no type");
    expect(() => useOf("blink", "nano-board", "", [])).toThrow(/^blink scene\.jsonc is missing|blink scene\.jsonc: /);
  });

  it("names a missing scene file", () => {
    const item: Item<BlockRecord> = {
      record: {
        format: 1,
        type: "block",
        name: "blink",
        version: "0.1.0",
        title: "blink",
        license: "MIT",
        board: "nano-board",
        registryDependencies: [],
        files: [],
        hash: "h",
      },
      versions: ["0.1.0"],
      usedBy: [],
    };
    expect(() => readCatalogUse([item], [], {})).toThrow("blink scene.jsonc is missing");
  });

  it("shows a fraction whole and a file param as its file name", () => {
    const resistor = partItem("resistor", { params: [{ name: "note" }] });
    const board = partItem("nano-board", { categories: ["board"], params: [{ name: "firmware", file: "hex" }] });
    const { circuits } = useOf(
      "fade",
      "nano-board",
      JSON.stringify({
        parts: {
          nano: { type: "nano-board", firmware: "firmware/fade.hex" },
          r1: { type: "resistor", note: "1/4" },
        },
      }),
      [resistor, board],
    );
    expect(circuits.fade?.placed.find((part) => part.ref === "r1")?.values).toEqual(["1/4"]);
    expect(circuits.fade?.placed.find((part) => part.ref === "nano")?.values).toEqual(["fade.hex"]);
  });

  it("shows a sketch's .h and .cpp and counts test files, not expect lines", () => {
    const extra = [
      file("firmware/fade/fade.ino", "ino"),
      file("firmware/fade/fade.cpp", "cpp"),
      file("firmware/fade/fade.h", "h"),
      file("firmware/fade.hex", "hex"),
      file("README.md", "readme"),
      file("tests/fade.test", "test"),
      file("tests/notes.txt", "notes"),
    ];
    const { circuits } = useOf(
      "fade",
      "nano-board",
      JSON.stringify({ parts: { nano: { type: "nano-board" } } }),
      [partItem("nano-board", { categories: ["board"] })],
      extra,
    );
    const sources = circuits.fade?.sources ?? [];
    expect(sources.filter((source) => source.group === "Sketch").map((source) => source.path)).toEqual([
      "firmware/fade/fade.cpp",
      "firmware/fade/fade.h",
      "firmware/fade/fade.ino",
    ]);
    expect(sources.filter((source) => source.group === "Scene").map((source) => source.path)).toEqual(["scene.jsonc"]);
    expect(sources.filter((source) => source.group === "Test").map((source) => source.path)).toEqual(["tests/fade.test", "tests/notes.txt"]);
    expect(circuits.fade?.tests).toBe(1);
  });

  it("counts a second test file", () => {
    const { circuits } = useOf(
      "fade",
      "nano-board",
      JSON.stringify({ parts: { nano: { type: "nano-board" } } }),
      [partItem("nano-board")],
      [file("tests/a.test", "a"), file("tests/b.test", "b")],
    );
    expect(circuits.fade?.tests).toBe(2);
  });
});

describe("glanceLine", () => {
  const board = { id: "nano-board", title: "Nano (CH340 clone)", href: "/embedded/parts/nano-board/" };
  const block = (id: string): PartUse["blocks"][number] => ({ id, title: id, href: `/embedded/blocks/${id}/`, refs: [] });

  it("drops the word the label already says", () => {
    const both: PartUse = { blocks: [block("a"), block("b"), block("c"), block("d")], boards: [board] };
    expect(glanceLine(both)).toEqual({ label: "used in", value: "4 Blocks", href: "#used" });
    expect(whereLine(both)).toBe("in 4 Blocks");
    const onBoard: PartUse = { blocks: [], boards: [board] };
    expect(glanceLine(onBoard)).toEqual({ label: "on boards", value: "Nano (CH340 clone)", href: "#boards" });
    expect(whereLine(onBoard)).toBe("on the Nano (CH340 clone)");
    expect(glanceLine({ blocks: [], boards: [] })).toEqual({ label: "used in", value: "no Block yet", href: undefined });
    expect(glanceLine({ blocks: [block("a")], boards: [] })).toEqual({ label: "used in", value: "1 Block", href: "#used" });
    expect(glanceLine({ blocks: [], boards: [board, { ...board, id: "uno-board", title: "Uno" }] }).value).toBe("2 boards");
  });
});

describe("lockSentences", () => {
  function circuit(placedRows: BlockCircuit["placed"], own: readonly string[], lockCount: number, title = "Nano (CH340 clone)"): BlockCircuit {
    return {
      placed: placedRows,
      board: { id: "nano-board", title, own: own.map((id) => ({ id, title: id })) },
      lockCount,
      tests: 1,
      sources: [],
    };
  }

  it("adds the repeated rows to the board's own parts", () => {
    const rows = [
      placed("rbot", "resistor-axial"),
      placed("rset", "resistor-axial"),
      placed("rhys", "resistor-axial"),
      placed("rled", "resistor-axial"),
      placed("bb", "breadboard-830"),
      placed("nano", "nano-board"),
      placed("u1", "lm358-dip8"),
      placed("ldr", "ldr-5mm"),
      placed("trim", "trimmer-3362p"),
      placed("led1", "led-5mm"),
    ];
    const own = Array.from({ length: 14 }, (_, index) => `child-${index}`);
    const { brings, lists } = lockSentences(circuit(rows, own, 21));
    expect(brings).toBe("The Nano (CH340 clone) brings 14 parts of its own.");
    expect(lists).toBe("The 10 rows above are 7 parts, because the 4 resistor-axial rows are one part, so parts.lock lists 21.");
  });

  it("states the count when every row is its own part", () => {
    const { brings, lists } = lockSentences(circuit([placed("a", "a"), placed("b", "b")], ["c", "d", "e"], 5, "Board"));
    expect(brings).toBe("The Board brings 3 parts of its own.");
    expect(lists).toBe("With the 2 parts above, parts.lock lists 5.");
  });

  it("says when a row is already one of the board's parts", () => {
    const { lists } = lockSentences(circuit([placed("nano", "nano-board"), placed("led", "led-0603")], ["led-0603", "cap", "resistor"], 4));
    expect(lists).toBe("The 2 rows above are 2 parts. One of those parts is already on the board. parts.lock lists 4.");
  });

  it("names every repeated part and still states a lock the rows do not add up to", () => {
    const rows = [placed("a1", "aaa"), placed("a2", "aaa"), placed("a3", "aaa"), placed("a4", "aaa"), placed("b1", "bbb"), placed("b2", "bbb"), placed("c", "ccc")];
    expect(lockSentences(circuit(rows, [], 3)).lists).toBe(
      "The 7 rows above are 3 parts, because the 4 aaa rows are one part and the 2 bbb rows are one part, so parts.lock lists 3.",
    );
    expect(lockSentences(circuit(rows, [], 99)).lists).toBe(
      "The 7 rows above are 3 parts, because the 4 aaa rows are one part and the 2 bbb rows are one part. parts.lock lists 99.",
    );
  });
});
