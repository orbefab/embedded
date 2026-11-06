import { Color } from "three";

/** The brightness reading's `full` current is intensity 1. */
export function glowIntensity(amps: number, full: number): number {
  if (!(amps > 0 && Number.isFinite(amps))) {
    return 0;
  }
  return amps / full;
}

/** `base` scaled to luminance 1, the bloom threshold, so every colour starts to bloom at
 * intensity 1. */
export function glowColour(base: Color): Color {
  const luminance = 0.2126 * base.r + 0.7152 * base.g + 0.0722 * base.b;
  return luminance > 0 ? base.clone().multiplyScalar(1 / luminance) : new Color(1, 1, 1);
}
