#include <Adafruit_NeoPixel.h>

#define PIN 6
#define NUMPIXELS 16

Adafruit_NeoPixel ring(NUMPIXELS, PIN, NEO_GRB + NEO_KHZ800);
uint16_t firstHue = 0;

void setup() {
  ring.begin();
}

void loop() {
  ring.rainbow(firstHue);
  ring.show();
  firstHue += 4096;
  delay(300);
}
