// Segments a to g on D2 to D8 and dp on D9. Each digit 0 to 9 shows for 100 ms with dp off,
// then 100 ms with dp on. Then dp alone shows for 100 ms, and the count starts again. D11 low
// drives a segment low to light it, for a common-anode display.
const byte glyph[10] = {0x3f, 0x06, 0x5b, 0x4f, 0x66, 0x6d, 0x7d, 0x07, 0x7f, 0x6f};
bool anode;

void show(byte bits) {
  for (int i = 0; i < 8; i++) digitalWrite(2 + i, ((bits >> i) & 1) != anode ? HIGH : LOW);
}

void setup() {
  pinMode(11, INPUT_PULLUP);
  anode = digitalRead(11) == LOW;
  for (int pin = 2; pin <= 9; pin++) pinMode(pin, OUTPUT);
}

void loop() {
  for (int n = 0; n < 10; n++) {
    show(glyph[n]);
    delay(100);
    show(glyph[n] | 0x80);
    delay(100);
  }
  show(0x80);
  delay(100);
}
