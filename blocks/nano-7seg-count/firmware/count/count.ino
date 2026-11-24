// a on D2, then b, c, d, e, f, g. Common cathode to GND. Each segment has its own resistor.
const int first = 2;

// Bits 0 to 6 are segments a to g.
const byte glyph[10] = {
  0x3f, // 0
  0x06, // 1
  0x5b, // 2
  0x4f, // 3
  0x66, // 4
  0x6d, // 5
  0x7d, // 6
  0x07, // 7
  0x7f, // 8
  0x6f, // 9
};

void setup() {
  for (int i = 0; i < 7; i++) pinMode(first + i, OUTPUT);
}

void show(int n) {
  byte bits = glyph[n];
  for (int i = 0; i < 7; i++) digitalWrite(first + i, (bits >> i) & 1 ? HIGH : LOW);
}

void loop() {
  for (int n = 0; n < 10; n++) {
    show(n);
    delay(200);
  }
}
