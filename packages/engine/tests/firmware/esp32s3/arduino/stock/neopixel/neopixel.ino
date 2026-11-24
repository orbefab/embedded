#include <Adafruit_NeoPixel.h>

Adafruit_NeoPixel strip(8, 4, NEO_GRB + NEO_KHZ800);

const uint32_t first[8] = {
  0xff0000, 0x00ff00, 0x0000ff, 0xffff00,
  0x00ffff, 0xff00ff, 0x102030, 0x804020,
};
const uint32_t second[8] = {
  0x010203, 0x400000, 0x004000, 0x000040,
  0x808080, 0x0a141e, 0xc0ffee, 0xffffff,
};

void show(const uint32_t *colors) {
  for (int i = 0; i < 8; i++) {
    strip.setPixelColor(i, colors[i]);
  }
  strip.show();
}

void setup() {
  Serial.begin(115200);
  strip.begin();
  show(first);
  Serial.println("first");
  delay(20);
  show(second);
  Serial.println("second");
}

void loop() {
  delay(1000);
}
