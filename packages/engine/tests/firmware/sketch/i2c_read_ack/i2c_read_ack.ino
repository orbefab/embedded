#include <Wire.h>

// Two of these share one bus and a target at 0x15. PD2 held low asks for two bytes, so the
// first is ACKed. Left high, it asks for one byte and NACKs it. Both wait to the same
// microsecond and read together. The one-byte master sends 1 on that ACK and reads the
// other's 0, so it loses arbitration. Each prints how many bytes requestFrom returned.
void setup() {
  pinMode(2, INPUT_PULLUP);
  byte count = digitalRead(2) == LOW ? 2 : 1;
  Serial.begin(115200);
  Wire.begin();
  while (micros() < 2000) {
  }
  byte got = Wire.requestFrom((uint8_t)0x15, count);
  Serial.print(count);
  Serial.print(" got ");
  Serial.println(got);
}

void loop() {}
