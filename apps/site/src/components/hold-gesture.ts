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

/** Holds the circuit button while Space, Enter or a pointer is down, and releases it when that
 * gesture ends or {@link HoldGesture.drop} runs. A key or pointer gesture also fires a trusted
 * click when it ends. That click is not a second press, including when a blur released the hold
 * before the key or pointer came up. A click that is not trusted, as a screen reader sends,
 * presses for a short time. */
export function holdGesture(target: HoldTarget): HoldGesture {
  let down = 0;
  let gesture = false;
  const keys = new Set<string>();
  const pointers = new Set<number>();

  const press = () => {
    down += 1;
    if (down === 1) target.press();
  };
  const releaseOne = () => {
    if (down === 0) return;
    down -= 1;
    if (down === 0) target.release();
  };
  const endIfIdle = () => {
    if (keys.size > 0 || pointers.size > 0) return;
    // The browser fires this gesture's trusted click after the keyup or pointerup microtasks
    // and before a timer. A script click can also arrive before that timer when the page is
    // busy. Leave the flag set until the timer so the trusted click is ignored. An untrusted
    // click does not wait for the timer.
    setTimeout(() => {
      if (keys.size === 0 && pointers.size === 0) gesture = false;
    }, 0);
  };

  return {
    keyDown(key, repeat) {
      if (key !== " " && key !== "Enter") return;
      if (repeat || keys.has(key)) return;
      keys.add(key);
      gesture = true;
      press();
    },
    keyUp(key) {
      if (!keys.delete(key)) return;
      releaseOne();
      endIfIdle();
    },
    pointerDown(id, button) {
      if (button !== 0 || pointers.has(id)) return;
      pointers.add(id);
      gesture = true;
      press();
    },
    pointerEnd(id) {
      if (!pointers.delete(id)) return;
      releaseOne();
      endIfIdle();
    },
    drop() {
      // The key or pointer may still be down. Keep it, so the click the browser owes this
      // gesture is ignored when it arrives. Forgetting the key here made that click look
      // like a screen-reader press: it held for 300 ms, which is too short to draw a frame
      // when the worker is busy, and it swallowed the next real click.
      if (keys.size > 0 || pointers.size > 0) gesture = true;
      if (down === 0) return;
      down = 0;
      target.release();
    },
    click(trusted) {
      if (down > 0) return;
      // The key or pointer already pressed. This click is the browser's, not a new one.
      if (trusted && gesture) {
        gesture = false;
        return;
      }
      press();
      setTimeout(releaseOne, 300);
    },
  };
}
