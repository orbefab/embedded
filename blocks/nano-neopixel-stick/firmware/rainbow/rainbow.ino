#include <Adafruit_NeoPixel.h>

#define PIN 6
#define NUMPIXELS 8

Adafruit_NeoPixel strip(NUMPIXELS, PIN, NEO_GRB + NEO_KHZ800);

void setup() {
  strip.begin();
  strip.rainbow();
  strip.show();
}

void loop() {
}
