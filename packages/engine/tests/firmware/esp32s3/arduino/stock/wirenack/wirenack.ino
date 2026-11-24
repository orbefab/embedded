#include <Wire.h>

// Scanner, 1-byte write, 200-byte write, 6-byte read and 40-byte read to an absent address 0x3C
// at 100 kHz and 400 kHz on a bus with pull-ups only.
void probe_all(const char *tag) {
  Wire.beginTransmission(0x3C);
  uint8_t scan = Wire.endTransmission();

  Wire.beginTransmission(0x3C);
  Wire.write(0x00);
  uint8_t w1 = Wire.endTransmission();

  Wire.beginTransmission(0x3C);
  for (int i = 0; i < 200; i++) {
    Wire.write((uint8_t)i);
  }
  uint8_t w200 = Wire.endTransmission();

  size_t r6 = Wire.requestFrom(0x3C, 6);
  size_t r40 = Wire.requestFrom(0x3C, 40);

  Serial.printf("%s scan %u w1 %u w200 %u r6 %u r40 %u\n", tag, scan, w1, w200, (unsigned)r6, (unsigned)r40);
}

void setup() {
  Serial.begin(115200);
  Wire.setBufferSize(256);
  Wire.begin();
  probe_all("k100");
  Wire.setClock(400000);
  probe_all("k400");
  Serial.println("done");
}

void loop() {
}
