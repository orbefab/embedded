/** The most UTF-8 bytes of serial text a page holds. A frame sends at most this many. */
export const SERIAL_HOLD = 4096;

/** The tail of the session's serial text a page shows, and how far it has read. */
export interface SerialView {
  readonly text: string;
  /** UTF-8 bytes of the session's serial text this view has caught up to. */
  readonly end: number;
}

export const EMPTY_SERIAL: SerialView = { text: "", end: 0 };

/** `serial` on a frame: new text at `from`, or a tail that replaces what a page shows. */
export interface SerialUpdate {
  readonly from: number;
  readonly text: string;
  readonly replace?: boolean;
}

/** Applies one frame's serial update. A `from` other than `view.end`, or `replace`, drops what
 * the page shows and keeps `text`. The page then holds at most {@link SERIAL_HOLD} bytes. */
export function applySerial(view: SerialView, update: SerialUpdate): SerialView {
  const added = utf8Length(update.text);
  const replace = update.replace === true || update.from !== view.end;
  const text = replace ? update.text : view.text + update.text;
  return { text: takeTail(text, SERIAL_HOLD), end: update.from + added };
}

function utf8Length(text: string): number {
  return new TextEncoder().encode(text).length;
}

/** The end of `text`, at most `max` UTF-8 bytes, on a character boundary. */
function takeTail(text: string, max: number): string {
  const bytes = new TextEncoder().encode(text);
  if (bytes.length <= max) return text;
  let start = bytes.length - max;
  while (start < bytes.length) {
    const byte = bytes[start];
    if (byte === undefined || (byte & 0xc0) !== 0x80) break;
    start += 1;
  }
  return new TextDecoder().decode(bytes.subarray(start));
}
