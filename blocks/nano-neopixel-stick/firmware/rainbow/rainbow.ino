#include <Adafruit_NeoPixel.h>

#define PIN 6
#define NUMPIXELS 8

Adafruit_NeoPixel strip(NUMPIXELS, PIN, NEO_GRB + NEO_KHZ800);
uint16_t firstHue = 0;

void setup() {
  strip.begin();
}

void loop() {
  strip.rainbow(firstHue);
  strip.show();
  firstHue += 8192;
  delay(800);
}
