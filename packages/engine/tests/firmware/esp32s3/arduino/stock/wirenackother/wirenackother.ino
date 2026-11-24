#include <Wire.h>

// Scanner, writes and reads to an absent address 0x3D while an OLED sits at 0x3C, then a scan,
// init and 65-byte write to the real OLED at 0x3C.
const uint8_t setupBytes[] = {0x00, 0x8D, 0x14, 0xAF};

void probe_missing(const char *tag) {
  Wire.beginTransmission(0x3D);
  uint8_t scan = Wire.endTransmission();

  Wire.beginTransmission(0x3D);
  Wire.write(0x00);
  uint8_t w1 = Wire.endTransmission();

  Wire.beginTransmission(0x3D);
  for (int i = 0; i < 200; i++) {
    Wire.write((uint8_t)i);
  }
  uint8_t w200 = Wire.endTransmission();

  size_t r6 = Wire.requestFrom(0x3D, 6);
  size_t r40 = Wire.requestFrom(0x3D, 40);

  Serial.printf("%s scan %u w1 %u w200 %u r6 %u r40 %u\n", tag, scan, w1, w200, (unsigned)r6, (unsigned)r40);
}

void setup() {
  Serial.begin(115200);
  // The OLED module holds RES# low through 4.7 kohm and 10 uF for about 76 ms after power-on.
  delay(100);
  Wire.setBufferSize(256);
  Wire.begin();
  probe_missing("k100");
  Wire.setClock(400000);
  probe_missing("k400");

  Wire.beginTransmission(0x3C);
  uint8_t found = Wire.endTransmission();
  Wire.beginTransmission(0x3C);
  Wire.write(setupBytes, sizeof setupBytes);
  uint8_t init = Wire.endTransmission();
  Wire.beginTransmission(0x3C);
  Wire.write(0x40);
  for (int i = 0; i < 64; i++) {
    Wire.write(0xFF);
  }
  uint8_t wrote = Wire.endTransmission();
  Serial.printf("oled scan %u init %u wrote %u\n", found, init, wrote);
}

void loop() {
}
