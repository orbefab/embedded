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
  it("presses for a click with no pointer or key before it, then releases", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.click();
    expect(events).toEqual(["press"]);
    vi.advanceTimersByTime(299);
    expect(events).toEqual(["press"]);
    vi.advanceTimersByTime(1);
    expect(events).toEqual(["press", "release"]);
  });

  it("presses once for a pointer, and ignores the click that follows", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(4, 0);
    hold.pointerEnd(4, true);
    hold.click();
    expect(events).toEqual(["press", "release"]);
    vi.advanceTimersByTime(0);
    hold.click();
    expect(events).toEqual(["press", "release", "press"]);
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("ignores the click that ends a pointer hold a blur cut short", () => {
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(1, 0);
    hold.drop();
    hold.pointerEnd(1, true);
    hold.click();
    expect(events).toEqual(["press", "release"]);
  });

  it("presses for a click after a key hold a blur cut short", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.keyDown(" ", false);
    hold.drop();
    hold.keyUp(" ");
    hold.click();
    expect(events).toEqual(["press", "release", "press"]);
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("a blur cancels a click's release, so a later held key stays down", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.click();
    vi.advanceTimersByTime(100);
    hold.drop();
    hold.keyDown(" ", false);
    vi.advanceTimersByTime(1000);
    expect(events).toEqual(["press", "release", "press"]);
    hold.keyUp(" ");
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("a click's release lets go of one hold, so a key pressed during it stays down", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.click();
    hold.keyDown(" ", false);
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press"]);
    hold.keyUp(" ");
    expect(events).toEqual(["press", "release"]);
  });

  it("ignores the click when a pointer leaves and lifts on the control, and a later click presses", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(1, 0);
    hold.pointerLeave(1);
    expect(events).toEqual(["press", "release"]);
    hold.pointerEnd(1, true);
    hold.click();
    expect(events).toEqual(["press", "release"]);
    vi.advanceTimersByTime(0);
    hold.click();
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("presses for a click after the pointer lifts off the control", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(1, 0);
    hold.pointerLeave(1);
    hold.pointerEnd(1, false);
    hold.click();
    expect(events).toEqual(["press", "release", "press"]);
    vi.advanceTimersByTime(300);
    expect(events).toEqual(["press", "release", "press", "release"]);
  });

  it("a blur after the pointer has left lets the next click press", () => {
    vi.useFakeTimers();
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(1, 0);
    hold.pointerLeave(1);
    hold.drop();
    hold.click();
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

  it("presses again for a pointer held, dropped, then pressed", () => {
    const { target, events } = recording();
    const hold = holdGesture(target);
    hold.pointerDown(1, 0);
    hold.drop();
    expect(events).toEqual(["press", "release"]);
    hold.pointerDown(1, 0);
    expect(events).toEqual(["press", "release", "press"]);
    hold.pointerEnd(1, false);
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
    hold.pointerEnd(1, false);
    expect(events).toEqual(["press", "release"]);
  });
});
