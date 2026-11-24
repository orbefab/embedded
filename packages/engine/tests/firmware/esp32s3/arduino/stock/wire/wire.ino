#include <Wire.h>

void setup() {
  Serial.begin(115200);
  Wire.begin();
  Serial.println("ready");
}

void loop() {
}
