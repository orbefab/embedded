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
  it("presses for a click with no key or pointer before it, then releases", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.click(true);
    expect(events).toEqual(["press"]);
    vi.advanceTimersByTime(299);
    expect(events).toEqual(["press"]);
    vi.advanceTimersByTime(1);
    expect(events).toEqual(["press", "release"]);
  });

  it("presses for an untrusted script click the same way", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.click(false);
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press", "release"]);
  });

  it("presses once for a pointer, and ignores the click that follows pointerup", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(4, 0);
    hold.pointerEnd(4);
    hold.click(true);
    expect(events).toEqual(["press", "release"]);
    vi.advanceTimersByTime(500);
    expect(events).toEqual(["press", "release"]);
  });

  it("ignores a trusted click that arrives in a later task, and presses after the window", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(4, 0);
    hold.pointerEnd(4);
    vi.advanceTimersByTime(0);
    hold.click(true);
    expect(events).toEqual(["press", "release"]);
    vi.advanceTimersByTime(500);
    hold.click(true);
    expect(events).toEqual(["press", "release", "press"]);
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("presses again for a key held, dropped, then pressed", () => {
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.keyDown(" ", false);
    hold.drop();
    expect(events).toEqual(["press", "release"]);
    hold.keyDown(" ", false);
    expect(events).toEqual(["press", "release", "press"]);
    hold.keyUp(" ");
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("presses again for pointer 1 held, dropped, then pressed", () => {
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(1, 0);
    hold.drop();
    expect(events).toEqual(["press", "release"]);
    // The pointerup happened in another window, so this id is still dropped.
    hold.pointerDown(1, 0);
    expect(events).toEqual(["press", "release", "press"]);
    hold.pointerEnd(1);
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("a pointerup for a dropped id clears it and does not press", () => {
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(1, 0);
    hold.drop();
    hold.pointerEnd(1);
    expect(events).toEqual(["press", "release"]);
  });

  it("ignores the click that ends a hold a blur cut short", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(1, 0);
    vi.advanceTimersByTime(100);
    hold.drop();
    vi.advanceTimersByTime(100);
    hold.pointerEnd(1);
    hold.click(true);
    vi.advanceTimersByTime(500);
    expect(events).toEqual(["press", "release"]);
  });

  it("ignores the click after a pointer leaves, comes back later and lifts", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(1, 0);
    vi.advanceTimersByTime(200);
    hold.pointerLeave(1);
    expect(events).toEqual(["press", "release"]);
    vi.advanceTimersByTime(1000);
    hold.pointerEnd(1);
    hold.click(true);
    vi.advanceTimersByTime(500);
    expect(events).toEqual(["press", "release"]);
  });

  it("a blur cancels a click's release, so a later held key stays down", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.click(true);
    vi.advanceTimersByTime(100);
    hold.drop();
    hold.keyDown(" ", false);
    vi.advanceTimersByTime(1000);
    expect(events).toEqual(["press", "release", "press"]);
    hold.keyUp(" ");
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("presses for a screen reader click after a blur", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.keyDown(" ", false);
    hold.drop();
    hold.click(true);
    expect(events).toEqual(["press", "release", "press"]);
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("a new key resets the previous gesture before its window ends", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.keyDown(" ", false);
    hold.keyUp(" ");
    vi.advanceTimersByTime(400);
    hold.keyDown("Enter", false);
    hold.keyUp("Enter");
    // The first gesture's 500 ms window would end here. The new gesture still owns the click.
    vi.advanceTimersByTime(100);
    hold.click(true);
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
