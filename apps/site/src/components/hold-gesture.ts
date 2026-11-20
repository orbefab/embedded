/** Press and release of the circuit button behind the hold control. */
export interface HoldTarget {
  press(): void;
  release(): void;
}

export interface HoldGesture {
  keyDown(key: string, repeat: boolean): void;
  keyUp(key: string): void;
  pointerDown(id: number, button: number): void;
  pointerEnd(id: number): void;
  /** The control blurred, the window blurred, or the page was hidden. */
  drop(): void;
  /** `trusted` is the click event's `isTrusted`. */
  click(trusted: boolean): void;
}

/** How long a finished key or pointer gesture still owns its trailing click. */
const GESTURE_MS = 500;
/** How long a click with no key or pointer holds the button. */
const CLICK_MS = 300;

/** Holds the circuit button while Space, Enter or a pointer is down, and releases it when that
 * gesture ends or {@link HoldGesture.drop} runs. A key or pointer gesture also fires a trusted
 * click. On Chrome for Android that click can arrive in a later task than touchend, so the
 * gesture stays set until the click arrives or for {@link GESTURE_MS}. The next key or pointer
 * starts a new gesture. VoiceOver, NVDA and TalkBack send a trusted click with no gesture, and
 * that click presses. A blur moves the held keys and pointers aside, so a later press or click
 * is a new one. */
export function holdGesture(target: HoldTarget): HoldGesture {
  let down = 0;
  let gesture = false;
  const keys = new Set<string>();
  const pointers = new Set<number>();
  const droppedKeys = new Set<string>();
  const droppedPointers = new Set<number>();
  let gestureTimer: ReturnType<typeof setTimeout> | undefined;

  const press = () => {
    down += 1;
    if (down === 1) target.press();
  };
  const releaseOne = () => {
    if (down === 0) return;
    down -= 1;
    if (down === 0) target.release();
  };
  const clearGestureTimer = () => {
    if (gestureTimer === undefined) return;
    clearTimeout(gestureTimer);
    gestureTimer = undefined;
  };
  const beginGesture = () => {
    clearGestureTimer();
    gesture = true;
  };
  const endIfIdle = () => {
    if (keys.size > 0 || pointers.size > 0) return;
    clearGestureTimer();
    gestureTimer = setTimeout(() => {
      gestureTimer = undefined;
      if (keys.size === 0 && pointers.size === 0) gesture = false;
    }, GESTURE_MS);
  };

  return {
    keyDown(key, repeat) {
      if (key !== " " && key !== "Enter") return;
      if (repeat || keys.has(key)) return;
      droppedKeys.delete(key);
      keys.add(key);
      beginGesture();
      press();
    },
    keyUp(key) {
      if (droppedKeys.delete(key)) {
        endIfIdle();
        return;
      }
      if (!keys.delete(key)) return;
      releaseOne();
      endIfIdle();
    },
    pointerDown(id, button) {
      if (button !== 0 || pointers.has(id)) return;
      droppedPointers.delete(id);
      pointers.add(id);
      beginGesture();
      press();
    },
    pointerEnd(id) {
      if (droppedPointers.delete(id)) {
        endIfIdle();
        return;
      }
      if (!pointers.delete(id)) return;
      releaseOne();
      endIfIdle();
    },
    drop() {
      for (const key of keys) droppedKeys.add(key);
      keys.clear();
      for (const id of pointers) droppedPointers.add(id);
      pointers.clear();
      clearGestureTimer();
      gesture = false;
      if (down === 0) return;
      down = 0;
      target.release();
    },
    click(trusted) {
      if (down > 0) return;
      if (trusted && gesture) {
        clearGestureTimer();
        gesture = false;
        return;
      }
      press();
      setTimeout(releaseOne, CLICK_MS);
    },
  };
}
