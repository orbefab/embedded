#include <Wire.h>

void setup() {
  Wire.begin();
  TWBR = 0;
  Wire.beginTransmission(0x15);
  Wire.write(0xA7);
  if (Wire.endTransmission(false) != 0) {
    return;
  }
  if (Wire.requestFrom(0x15, 1) != 1) {
    return;
  }
  Wire.read();
}

void loop() {}
