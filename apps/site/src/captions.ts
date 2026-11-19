/** What the page says a Block does. A Block in `blocks/` with no entry fails, rather than
 * showing its title and skipping the press control. */
export const CAPTIONS: Readonly<Record<string, string>> = {
  "nano-button-led": "Press the button and the LED lights. Let go and it goes out.",
  "uno-button-led": "Press the button and the LED lights, and the Uno's L LED with it. Let go and both go out.",
  "nano-led-fade": "The LED fades up and down on its own.",
  "nano-light-comparator": "Move the light slider. In the dark the LED lights.",
};

/** The caption for `name`. Throws when `blocks/` has a Block this map does not name. */
export function captionOf(name: string): string {
  const text = CAPTIONS[name];
  if (text === undefined) throw new Error(`${name} has no caption`);
  return text;
}
