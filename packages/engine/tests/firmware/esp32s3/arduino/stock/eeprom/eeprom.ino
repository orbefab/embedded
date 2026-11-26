#include <EEPROM.h>

void setup() {
  Serial.begin(115200);

  bool ok1 = EEPROM.begin(64);
  EEPROM.write(0, 0xA5);
  EEPROM.write(1, 0x5A);
  EEPROM.write(2, 0x11);
  EEPROM.write(3, 0x22);
  bool c1 = EEPROM.commit();
  EEPROM.end();

  bool ok2 = EEPROM.begin(64);
  uint8_t a0 = EEPROM.read(0);
  uint8_t a1 = EEPROM.read(1);
  uint8_t a2 = EEPROM.read(2);
  uint8_t a3 = EEPROM.read(3);
  EEPROM.write(0, 0x3C);
  bool c2 = EEPROM.commit();
  EEPROM.end();

  bool ok3 = EEPROM.begin(64);
  uint8_t b0 = EEPROM.read(0);
  uint8_t b1 = EEPROM.read(1);
  uint8_t b2 = EEPROM.read(2);
  uint8_t b3 = EEPROM.read(3);
  EEPROM.end();

  Serial.printf(
    "eeprom ok=%d%d%d commit=%d%d first=%02x,%02x,%02x,%02x second=%02x,%02x,%02x,%02x\n",
    ok1 ? 1 : 0,
    ok2 ? 1 : 0,
    ok3 ? 1 : 0,
    c1 ? 1 : 0,
    c2 ? 1 : 0,
    a0, a1, a2, a3,
    b0, b1, b2, b3
  );
}

void loop() {
  delay(100);
}
