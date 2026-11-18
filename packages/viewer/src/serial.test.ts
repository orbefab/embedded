import { describe, expect, test } from "vitest";
import { applySerial, EMPTY_SERIAL, SERIAL_HOLD, SERIAL_TRIMMED, serialReplay, serialShown } from "./serial";

describe("applySerial", () => {
  test("bytes append once and in order", () => {
    const first = applySerial(EMPTY_SERIAL, { from: 0, text: "light 1\r\n" });
    const second = applySerial(first, { from: first.end, text: "dark 2\r\n" });
    expect(second).toEqual({ text: "light 1\r\ndark 2\r\n", end: "light 1\r\ndark 2\r\n".length, trimmed: false });
    expect(serialShown(second)).toBe(second.text);
  });

  test("a replaced log drops what the session no longer has", () => {
    const shown = applySerial(EMPTY_SERIAL, { from: 0, text: "light 1\r\ndark 2\r\n" });
    const replaced = applySerial(shown, { from: 0, text: "light 1\r\n", replace: true });
    expect(replaced).toEqual({ text: "light 1\r\n", end: "light 1\r\n".length, trimmed: false });
  });

  test("a from behind the caught-up length replaces even without the flag", () => {
    const shown = applySerial(EMPTY_SERIAL, { from: 0, text: "abcd" });
    expect(applySerial(shown, { from: 0, text: "ab" })).toEqual({ text: "ab", end: 2, trimmed: false });
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
    expect(view.trimmed).toBe(true);
    expect(serialShown(view)).toBe(`${SERIAL_TRIMMED}\n${view.text}`);
  });

  test("a continuation longer than the hold arrives once, and the note marks the cut", () => {
    const head = "A".repeat(100);
    const body = "B".repeat(5000);
    const session = head + body + "C";
    let view = applySerial(EMPTY_SERIAL, { from: 0, text: head });
    view = applySerial(view, { from: head.length, text: body });
    expect(view.trimmed).toBe(true);
    expect(view.end).toBe(head.length + body.length);
    expect(view.text.length).toBeLessThanOrEqual(SERIAL_HOLD);
    expect(session.startsWith(head + body)).toBe(true);
    expect((head + body).endsWith(view.text)).toBe(true);
    expect(view.text.startsWith("B")).toBe(true);
    expect(serialShown(view)).toBe(`${SERIAL_TRIMMED}\n${view.text}`);
    expect(view.text.includes(SERIAL_TRIMMED)).toBe(false);

    view = applySerial(view, { from: view.end, text: "C" });
    expect(view.end).toBe(session.length);
    expect(view.trimmed).toBe(true);
    expect(session.endsWith(view.text)).toBe(true);
    expect(view.text.endsWith("C")).toBe(true);
    expect(serialShown(view).indexOf(SERIAL_TRIMMED)).toBe(0);
    expect(serialShown(view).indexOf(SERIAL_TRIMMED, 1)).toBe(-1);
  });

  test("a trimmed replace says what the join left out, and an empty replace clears it", () => {
    const tail = "B".repeat(100);
    const view = applySerial(EMPTY_SERIAL, { from: 1000, text: tail, replace: true, trimmed: true });
    expect(view).toEqual({ text: tail, end: 1100, trimmed: true });
    expect(serialShown(view)).toBe(`${SERIAL_TRIMMED}\n${tail}`);
    expect(serialReplay(view)).toEqual({ from: 1000, text: tail, replace: true, trimmed: true });
    expect(applySerial(view, { from: 0, text: "", replace: true })).toEqual(EMPTY_SERIAL);
  });
});
