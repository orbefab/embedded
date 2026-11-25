import { describe, expect, it } from "vitest";
import { glanceLine, readCatalogUse, whereLine, type PartUse } from "./circuit";
import type { BlockRecord, FileEntry, Item, PartRecord, ScenePart, SceneValue } from "./records";

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
  };
}

function blockItem(name: string, board: string, extra: readonly FileEntry[] = [], deps: readonly string[] = []): { item: Item<BlockRecord>; texts: Record<string, string> } {
  const sceneFile = file("scene.jsonc", `scene-${name}`);
  const texts: Record<string, string> = { [sceneFile.sha256]: "scene" };
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
    },
  };
}

function placed(ref: string, id: string, values: SceneValue[] = [], press = false): ScenePart {
  return { ref, part: id, press, values };
}

function useOf(
  name: string,
  board: string,
  scene: readonly ScenePart[],
  parts: readonly Item<PartRecord>[],
  extra: readonly FileEntry[] = [],
  deps: readonly string[] = [],
) {
  const { item, texts } = blockItem(name, board, extra, deps);
  return readCatalogUse([item], parts, texts, { [name]: scene });
}

describe("readCatalogUse", () => {
  it("keeps a part that a scene places and a board carries on both lists", () => {
    const led = partItem("led", { title: "LED" });
    const board = partItem("nano-board", { title: "Nano", categories: ["board"], registryDependencies: ["led@0.1.0"] });
    const { uses } = useOf("demo", "nano-board", [placed("nano", "nano-board"), placed("led1", "led")], [led, board], [], ["nano-board@0.1.0", "led@0.1.0"]);
    expect(uses.led?.blocks.map((block) => block.id)).toEqual(["demo"]);
    expect(uses.led?.boards.map((entry) => entry.id)).toEqual(["nano-board"]);
    expect(uses["nano-board"]?.blocks).toHaveLength(1);
    expect(uses["nano-board"]?.boards).toEqual([]);
  });

  it("leaves a board-only part off the Block list and a scene-only part off the board list", () => {
    const cap = partItem("cap");
    const resistor = partItem("resistor");
    const board = partItem("nano-board", { categories: ["board"], registryDependencies: ["cap@0.1.0"] });
    const { uses } = useOf("demo", "nano-board", [placed("nano", "nano-board"), placed("r1", "resistor")], [cap, resistor, board]);
    expect(uses.cap?.blocks).toEqual([]);
    expect(uses.cap?.boards.map((entry) => entry.title)).toEqual(["nano-board"]);
    expect(uses.resistor?.blocks).toHaveLength(1);
    expect(uses.resistor?.boards).toEqual([]);
  });

  it("fails the build when the scene does not place its board", () => {
    expect(() => useOf("blink", "nano-board", [placed("r1", "resistor")], [])).toThrow("blink scene.jsonc does not place its board nano-board");
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
    };
    expect(() => readCatalogUse([item], [], {}, {})).toThrow("blink scene.jsonc is missing");
  });

  it("shows a fraction whole and a file param as its file name", () => {
    const { circuits } = useOf(
      "fade",
      "nano-board",
      [
        placed("nano", "nano-board", [{ name: "firmware", file: "build/fade.ino.hex" }]),
        placed("r1", "resistor", [{ name: "note", text: "1/4" }]),
      ],
      [partItem("resistor"), partItem("nano-board", { categories: ["board"] })],
    );
    expect(circuits.fade?.placed.find((part) => part.ref === "r1")?.values).toEqual(["1/4"]);
    expect(circuits.fade?.placed.find((part) => part.ref === "nano")?.values).toEqual(["fade.ino.hex"]);
  });

  it("shows a trimmer's resistance, then its position by name", () => {
    const { circuits } = useOf(
      "light",
      "nano-board",
      [
        placed("nano", "nano-board"),
        placed("trim", "trimmer", [
          { name: "resistance", amount: 10000, unit: "ohm" },
          { name: "position", amount: 0.5, unit: "1" },
        ]),
      ],
      [partItem("nano-board"), partItem("trimmer")],
    );
    expect(circuits.light?.placed.find((part) => part.ref === "trim")?.values).toEqual(["10 kΩ", "position 0.5"]);
  });

  it("holds the first placed part with a press input, in scene order", () => {
    const { circuits } = useOf(
      "demo",
      "nano-board",
      [placed("nano", "nano-board"), placed("btn1", "switch", [], true), placed("sw", "switch", [], true)],
      [partItem("nano-board"), partItem("switch")],
    );
    expect(circuits.demo?.hold).toBe("@btn1");
    const quiet = useOf("quiet", "nano-board", [placed("nano", "nano-board")], [partItem("nano-board")]);
    expect(quiet.circuits.quiet?.hold).toBeNull();
  });

  it("shows a sketch's .h and .cpp and counts test files, not expect lines", () => {
    const extra = [
      file("firmware/fade/fade.ino", "ino"),
      file("firmware/fade/fade.cpp", "cpp"),
      file("firmware/fade/fade.h", "h"),
      file("build/fade.ino.hex", "hex"),
      file("README.md", "readme"),
      file("tests/fade.test", "test"),
      file("tests/notes.txt", "notes"),
    ];
    const { circuits } = useOf("fade", "nano-board", [placed("nano", "nano-board")], [partItem("nano-board", { categories: ["board"] })], extra);
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
    const { circuits } = useOf("fade", "nano-board", [placed("nano", "nano-board")], [partItem("nano-board")], [
      file("tests/a.test", "a"),
      file("tests/b.test", "b"),
    ]);
    expect(circuits.fade?.tests).toBe(2);
  });
});

describe("glanceLine", () => {
  const board = { id: "nano-board", title: "Nano (CH340 clone)", href: "/orbe/parts/nano-board/" };
  const block = (id: string): PartUse["blocks"][number] => ({ id, title: id, href: `/orbe/blocks/${id}/`, refs: [] });

  it("drops the word the label already says", () => {
    const both: PartUse = { blocks: [block("a"), block("b"), block("c"), block("d")], boards: [board] };
    expect(glanceLine(both)).toEqual({ label: "used in", value: "4 Blocks", href: "#used" });
    expect(whereLine(both)).toBe("in 4 Blocks");
    const onBoard: PartUse = { blocks: [], boards: [board] };
    expect(glanceLine(onBoard)).toEqual({ label: "on boards", value: "Nano (CH340 clone)", href: "#boards" });
    expect(whereLine(onBoard)).toBe("on the Nano (CH340 clone)");
    expect(glanceLine({ blocks: [], boards: [] })).toBeUndefined();
    expect(whereLine({ blocks: [], boards: [] })).toBe("");
    expect(glanceLine({ blocks: [block("a")], boards: [] })).toEqual({ label: "used in", value: "1 Block", href: "#used" });
    const many = glanceLine({ blocks: [], boards: [board, { ...board, id: "uno-board", title: "Uno" }] });
    expect(many?.value).toBe("2 boards");
  });
});
