import { describe, expect, test } from "vitest";
import { type Get, loadBlock } from "./registry";

const BASE = "https://example.test/r/";
const text = (value: unknown) => new TextEncoder().encode(typeof value === "string" ? value : JSON.stringify(value));
const hex = (digest: ArrayBuffer) => [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
const sha = async (bytes: Uint8Array) => hex(await crypto.subtle.digest("SHA-256", bytes as Uint8Array<ArrayBuffer>));

interface Item {
  type: string;
  name: string;
  version: string;
  deps?: string[];
  files: Record<string, string>;
}

/** A registry as `orbe registry build` writes one, by URL, with `edit` applied to each record. */
async function registry(items: Item[], edit: (record: Record<string, unknown>) => void = () => {}): Promise<Map<string, Uint8Array>> {
  const served = new Map<string, Uint8Array>();
  const rows = [];
  for (const item of items) {
    const files = [];
    for (const [path, body] of Object.entries(item.files).sort(([a], [b]) => (a < b ? -1 : 1))) {
      const bytes = text(body);
      const sha256 = await sha(bytes);
      served.set(`${BASE}files/${sha256}`, bytes);
      files.push({ path, url: `files/${sha256}`, sha256, bytes: bytes.byteLength, license: "MIT" });
    }
    const lines = files.map((file) => `${file.sha256}  ${file.path}\n`).join("");
    const record: Record<string, unknown> = {
      type: item.type,
      format: 1,
      name: item.name,
      version: item.version,
      registryDependencies: item.deps ?? [],
      files,
      hash: `sha256:${await sha(text(lines))}`,
    };
    edit(record);
    const bytes = text(record);
    const url = `${item.name}@${item.version}.json`;
    served.set(`${BASE}${url}`, bytes);
    rows.push({ name: item.name, type: item.type, version: item.version, url, sha256: await sha(bytes), bytes: bytes.byteLength });
  }
  served.set(`${BASE}index.json`, text({ format: 1, name: "@test", items: rows }));
  return served;
}

const getFrom =
  (served: Map<string, Uint8Array>): Get =>
  async (url) => {
    const bytes = served.get(url.href);
    if (bytes === undefined) throw new Error(`${url.href} answered 404`);
    return bytes;
  };

const index = new URL("index.json", BASE);
const led = (version: string): Item => ({
  type: "part",
  name: "led-5mm",
  version,
  files: { "part.jsonc": `{"version": "${version}"}`, "led-5mm.glb": "glTF" },
});
const block = (deps: string[]): Item => ({
  type: "block",
  name: "blink",
  version: "0.1.0",
  deps,
  files: { "block.jsonc": "{}", "scene.jsonc": "{}", "firmware/blink.hex": ":00000001FF" },
});

describe("loadBlock", () => {
  test("gives the Block's files and each part's at parts/<id>/, at the version the Block names", async () => {
    const served = await registry([block(["led-5mm@0.1.0"]), led("0.1.0"), led("0.2.0")]);
    const got = await loadBlock(index, "blink", getFrom(served));
    expect(got.name).toBe("blink");
    expect([...got.files.keys()].sort()).toEqual([
      "block.jsonc",
      "firmware/blink.hex",
      "parts/led-5mm/led-5mm.glb",
      "parts/led-5mm/part.jsonc",
      "scene.jsonc",
    ]);
    expect(new TextDecoder().decode(got.files.get("parts/led-5mm/part.jsonc"))).toBe('{"version": "0.1.0"}');
  });

  test("refuses a file whose bytes are not the ones its record lists", async () => {
    const served = await registry([block(["led-5mm@0.1.0"]), led("0.1.0")]);
    const glb = await sha(text("glTF"));
    served.set(`${BASE}files/${glb}`, text("gltf"));
    await expect(loadBlock(index, "blink", getFrom(served))).rejects.toThrow(/led-5mm\.glb at .* has SHA-256 [0-9a-f]{64}, not /);
    served.set(`${BASE}files/${glb}`, text("glTF!"));
    await expect(loadBlock(index, "blink", getFrom(served))).rejects.toThrow(/is 5 bytes, not 4/);
  });

  test("refuses a record that is not the one its index row lists", async () => {
    const served = await registry([block([])]);
    served.set(`${BASE}blink@0.1.0.json`, text({ ...JSON.parse(new TextDecoder().decode(served.get(`${BASE}blink@0.1.0.json`))), title: "x" }));
    await expect(loadBlock(index, "blink", getFrom(served))).rejects.toThrow(/blink's record at .* is \d+ bytes, not \d+/);
  });

  test("refuses a record whose files do not hash to its hash", async () => {
    const served = await registry([block([])], (record) => {
      record.hash = `sha256:${"0".repeat(64)}`;
    });
    await expect(loadBlock(index, "blink", getFrom(served))).rejects.toThrow(/blink@0\.1\.0\.json's files hash to sha256:[0-9a-f]{64}, not sha256:0{64}/);
  });

  test("refuses a path some platform cannot hold, and two paths that differ only in case", async () => {
    for (const path of ["../x", "/x", "a//b", "a\\b", "con.txt", "x.", "a:b", "x\u0001"]) {
      const served = await registry([{ ...block([]), files: { [path]: "x" } }]);
      await expect(loadBlock(index, "blink", getFrom(served)), path).rejects.toThrow(/which some platform cannot hold/);
    }
    const served = await registry([{ ...block([]), files: { "Scene.jsonc": "{}", "scene.jsonc": "{}" } }]);
    await expect(loadBlock(index, "blink", getFrom(served))).rejects.toThrow(/twice, case aside/);
  });

  test("refuses a file served at another URL than files/<sha256>", async () => {
    const served = await registry([block([])], (record) => {
      const files = record.files as { url: string }[];
      if (files[0] !== undefined) files[0].url = "elsewhere";
    });
    await expect(loadBlock(index, "blink", getFrom(served))).rejects.toThrow(/puts block\.jsonc at elsewhere/);
  });

  test("refuses an index that lists one version twice, a missing Block or part, and another registry's part", async () => {
    const twice = await registry([block([]), block([])]);
    await expect(loadBlock(index, "blink", getFrom(twice))).rejects.toThrow("the index lists blink@0.1.0 twice");
    const missing = await registry([block(["led-5mm@0.3.0"]), led("0.1.0")]);
    await expect(loadBlock(index, "nope", getFrom(missing))).rejects.toThrow("the registry has no nope");
    await expect(loadBlock(index, "blink", getFrom(missing))).rejects.toThrow("the registry has no led-5mm@0.3.0");
    await expect(loadBlock(index, "led-5mm", getFrom(missing))).rejects.toThrow("led-5mm is a part, not a block");
    const foreign = await registry([block(["@acme/lamp@1.0.0"])]);
    await expect(loadBlock(index, "blink", getFrom(foreign))).rejects.toThrow("blink runs @acme/lamp@1.0.0, from another registry");
  });
});
