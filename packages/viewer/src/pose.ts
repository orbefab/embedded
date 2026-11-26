// Poses a built scene as one frame of its session shows it: each LED's glow, each screen's
// glass, each switch's slide and each turn joint, such as a servo horn or a knob. The viewer runs
// it on every drawn frame, and the site's Block pictures run it once on the frame they draw.
import { Quaternion } from "three";
import type { BuiltScene } from "./build";
import { glowIntensity, litColour, pixelGlow, shownCurrent } from "./glow";
import type { FrameMessage, ScreenImage } from "./protocol";
import { showScreen } from "./screen";

/** Scratch for a turn joint's pose, so a frame allocates nothing. */
const turned = new Quaternion();

/** Poses `built` as `live` shows it, or at rest when `live` is null. `peaks` is each LED
 * channel's brightest current since the previous drawn frame, and is reset to `live`'s. `screens`
 * is each screen's last glass, per `channels.screens`. */
export function poseFrame(
  built: BuiltScene,
  live: FrameMessage | null,
  peaks: number[],
  screens: readonly (ScreenImage | undefined)[]
): void {
  for (const glow of built.glows) {
    const { by } = glow;
    const intensity =
      by.kind === "current"
        ? glowIntensity(shownCurrent(live?.led_a[by.channel] ?? 0, peaks[by.channel] ?? 0), by.full)
        : pixelGlow(live?.pixel_rgb[by.channel] ?? [], glow.material.emissive);
    glow.material.emissiveIntensity = intensity;
    litColour(glow.base, intensity, glow.material.color);
  }
  const count = Math.max(peaks.length, live?.led_a.length ?? 0);
  for (let channel = 0; channel < count; channel += 1) peaks[channel] = live?.led_a[channel] ?? 0;
  peaks.length = count;
  for (const screen of built.screens) {
    const image = screens[screen.channel];
    if (image !== undefined && image !== screen.shown) showScreen(screen, image);
  }
  for (const slide of built.slides) {
    slide.node.position.copy(slide.rest);
    if (live?.switches[slide.channel] === 1) slide.node.position.add(slide.delta);
  }
  for (const turn of built.turns) {
    const value = turn.horn ? (live?.horn_deg?.[turn.channel] ?? turn.value) : (live?.params?.[turn.channel] ?? turn.value);
    turned.setFromAxisAngle(turn.axis, turn.angle(value));
    turn.node.quaternion.copy(turned).multiply(turn.rest);
  }
}
