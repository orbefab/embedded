#include <Wire.h>

// Probes every 7-bit address from 1 to 126 at 400 kHz and prints each one that answered.
void setup() {
  Serial.begin(115200);
  Wire.begin();
  Wire.setClock(400000);
  byte found = 0;
  for (byte address = 1; address < 127; address++) {
    Wire.beginTransmission(address);
    byte error = Wire.endTransmission();
    if (error == 0) {
      Serial.print("found 0x");
      Serial.println(address, HEX);
      found++;
    } else if (error != 2) {
      Serial.print("error ");
      Serial.print(error);
      Serial.print(" at 0x");
      Serial.println(address, HEX);
    }
  }
  if (found == 0) {
    Serial.println("no devices");
  }
}

void loop() {}
