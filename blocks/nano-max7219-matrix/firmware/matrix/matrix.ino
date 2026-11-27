#include <LedControl.h>

// DIN = D11, CLK = D13, CS = D10, 1 MAX7219 module
LedControl lc(11, 13, 10, 1);

static const byte SMILEY[8] = {
  0b00111100,
  0b01000010,
  0b10100101,
  0b10000001,
  0b10100101,
  0b10011001,
  0b01000010,
  0b00111100,
};

void setup() {
  lc.shutdown(0, false);
  lc.setIntensity(0, 8);
  lc.clearDisplay(0);
  for (int row = 0; row < 8; row++) {
    lc.setRow(0, row, SMILEY[row]);
  }
}

void loop() {}
