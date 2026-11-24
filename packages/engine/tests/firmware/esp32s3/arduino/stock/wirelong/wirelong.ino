#include <Wire.h>

// One write of 1 + 64 bytes and one read of 40 bytes to the OLED module at 0x3C, each a single
// transaction longer than the 32-byte I2C FIFO.
const uint8_t setupBytes[] = {0x00, 0x8D, 0x14, 0xAF};

void setup() {
  Serial.begin(115200);
  delay(100);
  Wire.begin();
  Wire.beginTransmission(0x3C);
  Wire.write(setupBytes, sizeof setupBytes);
  Serial.printf("init %u\n", Wire.endTransmission());
  Wire.beginTransmission(0x3C);
  Wire.write(0x40);
  for (int i = 0; i < 64; i++) {
    Wire.write(0xFF);
  }
  Serial.printf("wrote %u\n", Wire.endTransmission());
  uint8_t n = Wire.requestFrom(0x3C, 40);
  int ff = 0;
  while (Wire.available()) {
    ff += Wire.read() == 0xFF;
  }
  Serial.printf("read %u ff %d\n", n, ff);
}

void loop() {
}
