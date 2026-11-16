/** Positions on a quantity's thumb. The ends are its minimum and its maximum. */
export const WORLD_STEPS = 1000;

export interface Range {
  readonly min: number;
  readonly max: number;
}

/** A thumb is logarithmic when its minimum is at least 0 and its maximum is at least 1000, so a
 * small value stays reachable. Temperature and humidity stay linear. */
export function logRange(range: Range): boolean {
  return range.min >= 0 && range.max >= 1000;
}

function quantum(range: Range): number {
  const span = range.max - range.min;
  if (span >= 1000) return 1;
  if (span >= 20) return 0.1;
  return 0.01;
}

/** `value` snapped onto the thumb's quantum and held inside the range. */
export function snap(range: Range, value: number): number {
  const step = quantum(range);
  const digits = step >= 1 ? 0 : step >= 0.1 ? 1 : 2;
  const snapped = Number((Math.round(value / step) * step).toFixed(digits));
  return Math.min(range.max, Math.max(range.min, snapped));
}

/** The thumb position, 0 to {@link WORLD_STEPS}, whose snapped value is `value`, or the nearest. */
export function stepOf(range: Range, value: number): number {
  const target = snap(range, value);
  let lo = 0;
  let hi = WORLD_STEPS;
  while (lo <= hi) {
    const mid = Math.floor((lo + hi) / 2);
    const found = valueAt(range, mid);
    if (found === target) return mid;
    if (found < target) lo = mid + 1;
    else hi = mid - 1;
  }
  const candidates = [lo - 1, lo].filter((step) => step >= 0 && step <= WORLD_STEPS);
  return candidates.reduce((best, step) =>
    Math.abs(valueAt(range, step) - target) < Math.abs(valueAt(range, best) - target) ? step : best,
  );
}

/** The snapped value at thumb position `step`. */
export function valueAt(range: Range, step: number): number {
  const t = Math.min(WORLD_STEPS, Math.max(0, step)) / WORLD_STEPS;
  if (!logRange(range)) return snap(range, range.min + t * (range.max - range.min));
  const start = Math.log1p(range.min);
  const end = Math.log1p(range.max);
  return snap(range, Math.expm1(start + t * (end - start)));
}

/** `value` in the digits the thumb snaps to, without trailing zeros. */
export function formatQuantity(range: Range, value: number): string {
  const step = quantum(range);
  const digits = step >= 1 ? 0 : step >= 0.1 ? 1 : 2;
  const text = snap(range, value).toFixed(digits);
  return digits === 0 ? text : text.replace(/0+$/, "").replace(/\.$/, "");
}
