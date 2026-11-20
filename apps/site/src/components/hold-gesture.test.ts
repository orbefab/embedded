import { afterEach, describe, expect, it, vi } from "vitest";
import { holdGesture, type HoldTarget } from "./hold-gesture";

function recording(): { target: HoldTarget; events: string[] } {
  const events: string[] = [];
  return {
    events,
    target: {
      press: () => events.push("press"),
      release: () => events.push("release"),
    },
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("hold gesture", () => {
  it("presses for a click that had no key or pointer, then releases", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.click(false);
    expect(events).toEqual(["press"]);
    vi.advanceTimersByTime(299);
    expect(events).toEqual(["press"]);
    vi.advanceTimersByTime(1);
    expect(events).toEqual(["press", "release"]);
  });

  it("presses once for a pointer, and ignores the click that follows pointerup", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(4, 0);
    hold.pointerEnd(4);
    // The browser's click is trusted and arrives before the timer that clears the gesture.
    hold.click(true);
    expect(events).toEqual(["press", "release"]);
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press", "release"]);
  });

  it("releases on blur and still presses for a later click", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.keyDown(" ", false);
    expect(events).toEqual(["press"]);
    hold.drop();
    expect(events).toEqual(["press", "release"]);
    hold.keyUp(" ");
    // Space's own click, when the browser sends one, is trusted and is not a new press.
    hold.click(true);
    expect(events).toEqual(["press", "release"]);
    // A script click can arrive before the timer when the page is busy. It still presses.
    hold.click(false);
    expect(events).toEqual(["press", "release", "press"]);
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("presses for a trusted click once the gesture's timer has cleared it", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.keyDown(" ", false);
    hold.keyUp(" ");
    vi.advanceTimersByTime(0);
    hold.click(true);
    expect(events).toEqual(["press", "release", "press"]);
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("ignores a repeated key and a second pointer with the same id", () => {
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.keyDown(" ", false);
    hold.keyDown(" ", true);
    hold.pointerDown(1, 0);
    hold.pointerDown(1, 0);
    expect(events).toEqual(["press"]);
    hold.keyUp(" ");
    expect(events).toEqual(["press"]);
    hold.pointerEnd(1);
    expect(events).toEqual(["press", "release"]);
  });
});
