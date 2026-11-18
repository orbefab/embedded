import { describe, expect, test } from "vitest";
import { applySerial, EMPTY_SERIAL, SERIAL_HOLD } from "./serial";

describe("applySerial", () => {
  test("bytes append once and in order", () => {
    const first = applySerial(EMPTY_SERIAL, { from: 0, text: "light 1\r\n" });
    const second = applySerial(first, { from: first.end, text: "dark 2\r\n" });
    expect(second.text).toBe("light 1\r\ndark 2\r\n");
    expect(second.end).toBe("light 1\r\ndark 2\r\n".length);
  });

  test("a replaced log drops what the session no longer has", () => {
    const shown = applySerial(EMPTY_SERIAL, { from: 0, text: "light 1\r\ndark 2\r\n" });
    const replaced = applySerial(shown, { from: 0, text: "light 1\r\n", replace: true });
    expect(replaced).toEqual({ text: "light 1\r\n", end: "light 1\r\n".length });
  });

  test("a from behind the caught-up length replaces even without the flag", () => {
    const shown = applySerial(EMPTY_SERIAL, { from: 0, text: "abcd" });
    expect(applySerial(shown, { from: 0, text: "ab" })).toEqual({ text: "ab", end: 2 });
  });

  test("an empty replace clears the text", () => {
    const shown = applySerial(EMPTY_SERIAL, { from: 0, text: "dark 2\r\n" });
    expect(applySerial(shown, { from: 0, text: "", replace: true })).toEqual(EMPTY_SERIAL);
  });

  test("a page holds the tail and keeps the cut on a character", () => {
    const chunk = `${"é".repeat(3000)}z`;
    const view = applySerial(EMPTY_SERIAL, { from: 0, text: chunk, replace: true });
    const bytes = new TextEncoder().encode(view.text);
    expect(view.end).toBe(new TextEncoder().encode(chunk).length);
    expect(bytes.length).toBeLessThanOrEqual(SERIAL_HOLD);
    expect(chunk.endsWith(view.text)).toBe(true);
    expect(view.text.startsWith("é")).toBe(true);
  });
});
