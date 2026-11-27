#include <MD_Parola.h>
#include <MD_MAX72xx.h>
#include <SPI.h>

#define HARDWARE_TYPE MD_MAX72XX::FC16_HW
#define MAX_DEVICES 4
#define CS_PIN 10

MD_Parola P(HARDWARE_TYPE, CS_PIN, MAX_DEVICES);

void setup() {
  P.begin();
  P.setIntensity(6);
  P.displayText("ORBE", PA_CENTER, 20, 0, PA_SCROLL_LEFT, PA_SCROLL_LEFT);
}

void loop() {
  if (P.displayAnimate()) {
    P.displayReset();
  }
}
