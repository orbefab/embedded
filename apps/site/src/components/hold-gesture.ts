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
  /** The pointer left the control while down. Its pointerup can still land on the control. */
  pointerLeave(id: number): void;
  /** The control blurred, the window blurred, or the page was hidden. */
  drop(): void;
  click(): void;
}

/** How long a click with no pointer or key before it holds the button. */
const CLICK_MS = 300;

/** Holds the circuit button while Space, Enter or a pointer is down, and releases it when that
 * gesture ends or {@link HoldGesture.drop} runs. The click that follows a pointer or a key is
 * ignored, so the browser's extra click does not press twice. A click with no pointer or key
 * before it, such as from a screen reader, presses for {@link CLICK_MS}. A blur while a pointer
 * is still down keeps that click ignored. A blur while only a key is down does not. */
export function holdGesture(target: HoldTarget): HoldGesture {
  let down = 0;
  const keys = new Set<string>();
  const pointers = new Set<number>();
  /** A pointer or a key already pressed, so the click that follows is not another press. */
  let oweClick = false;
  let clickTimer: ReturnType<typeof setTimeout> | undefined;

  const press = () => {
    down += 1;
    if (down === 1) target.press();
  };
  const releaseOne = () => {
    if (down === 0) return;
    down -= 1;
    if (down === 0) target.release();
  };
  const cancelClick = () => {
    if (clickTimer === undefined) return;
    clearTimeout(clickTimer);
    clickTimer = undefined;
  };

  return {
    keyDown(key, repeat) {
      if (key !== " " && key !== "Enter") return;
      if (repeat || keys.has(key)) return;
      keys.add(key);
      oweClick = true;
      press();
    },
    keyUp(key) {
      if (!keys.delete(key)) return;
      releaseOne();
    },
    pointerDown(id, button) {
      if (button !== 0 || pointers.has(id)) return;
      pointers.add(id);
      oweClick = true;
      press();
    },
    pointerEnd(id) {
      if (!pointers.delete(id)) return;
      releaseOne();
    },
    pointerLeave(id) {
      if (!pointers.delete(id)) return;
      oweClick = true;
      releaseOne();
    },
    drop() {
      // A pointer that was still down owns the click that follows. A key does not, and neither
      // does a pointer that had already left.
      oweClick = pointers.size > 0;
      keys.clear();
      pointers.clear();
      cancelClick();
      if (down === 0) return;
      down = 0;
      target.release();
    },
    click() {
      if (oweClick || down > 0) {
        oweClick = false;
        return;
      }
      press();
      clickTimer = setTimeout(() => {
        clickTimer = undefined;
        releaseOne();
      }, CLICK_MS);
    },
  };
}
