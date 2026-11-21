// Adapted from maker-sandbox (MIT, Copyright 2026 Alwurts).
import { Color } from "three";

/** The brightness reading's `full` current is intensity 1. */
export function glowIntensity(amps: number, full: number): number {
  if (!(amps > 0 && Number.isFinite(amps))) {
    return 0;
  }
  return amps / full;
}

/** `base` scaled to luminance 1, the bloom threshold. */
export function glowColour(base: Color): Color {
  const luminance = 0.2126 * base.r + 0.7152 * base.g + 0.0722 * base.b;
  return luminance > 0 ? base.clone().multiplyScalar(1 / luminance) : new Color(1, 1, 1);
}

/** The surface colour under a glow at `intensity`: `base` at 0, fading to black at 1. The
 * light on the surface then adds nothing at intensity 1, so every colour starts to bloom
 * there. */
export function litColour(base: Color, intensity: number, into: Color): Color {
  return into.copy(base).multiplyScalar(Math.max(0, 1 - intensity));
}

/** The current to draw. `peak` is the brightest current since the previous drawn frame.
 * A pulse that has already fallen still shows once, so a short press cannot land between
 * frames and disappear. A current that is still near that peak draws as it is. */
export function shownCurrent(live: number, peak: number): number {
  return live < peak * 0.5 ? peak : live;
}
