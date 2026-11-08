#include <Wire.h>

// Reads register 0x02 for five bytes and prints the finger count and the point.
void readPoint() {
  Wire.beginTransmission(0x15);
  Wire.write(0x02);
  if (Wire.endTransmission(false) != 0) {
    return;
  }
  if (Wire.requestFrom(0x15, 5) != 5) {
    return;
  }
  byte fingers = Wire.read();
  byte xh = Wire.read();
  byte xl = Wire.read();
  byte yh = Wire.read();
  byte yl = Wire.read();
  unsigned int x = (unsigned int)(xh & 0x0F) << 8 | xl;
  unsigned int y = (unsigned int)(yh & 0x0F) << 8 | yl;
  Serial.print(fingers);
  Serial.print(' ');
  Serial.print(x);
  Serial.print(' ');
  Serial.println(y);
}

void setup() {
  Serial.begin(9600);
  Wire.begin();
  Wire.setClock(100000);
  delay(20);
  readPoint();
  delay(20);
  readPoint();
}

void loop() {}
