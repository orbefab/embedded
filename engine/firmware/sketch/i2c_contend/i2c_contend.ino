#include <Wire.h>

// Two of these share one bus. PD2 held low picks address 0x50 and left high picks 0x51. Both
// wait to the same microsecond and address their own at once, so 0x51's master loses the
// arbitration at the last address bit. Each prints what endTransmission returned.
void setup() {
  pinMode(2, INPUT_PULLUP);
  byte address = digitalRead(2) == LOW ? 0x50 : 0x51;
  Serial.begin(115200);
  Wire.begin();
  while (micros() < 2000) {
  }
  Wire.beginTransmission(address);
  byte error = Wire.endTransmission();
  Serial.print("0x");
  Serial.print(address, HEX);
  Serial.print(" returned ");
  Serial.println(error);
}

void loop() {}
