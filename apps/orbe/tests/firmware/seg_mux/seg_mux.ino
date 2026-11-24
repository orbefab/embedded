// Segments a to g on D2 to D8, common cathode on D10. Each 10 ms frame writes the digit with the
// cathode high, then pulls it low for 2.5 ms. Each digit holds 20 frames, counting 0 to 9.
const byte glyph[10] = {0x3f, 0x06, 0x5b, 0x4f, 0x66, 0x6d, 0x7d, 0x07, 0x7f, 0x6f};

void setup() {
  for (int pin = 2; pin <= 8; pin++) pinMode(pin, OUTPUT);
  pinMode(10, OUTPUT);
  digitalWrite(10, HIGH);
}

void loop() {
  for (int n = 0; n < 10; n++) {
    for (int frame = 0; frame < 20; frame++) {
      for (int i = 0; i < 7; i++) digitalWrite(2 + i, (glyph[n] >> i) & 1 ? HIGH : LOW);
      digitalWrite(10, LOW);
      delayMicroseconds(2500);
      digitalWrite(10, HIGH);
      delayMicroseconds(7500);
    }
  }
}
