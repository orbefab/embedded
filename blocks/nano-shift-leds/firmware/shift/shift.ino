// Walk one lit LED from QA to QH and back. QA is the least significant bit.
const int dataPin = 11;
const int clockPin = 12;
const int latchPin = 8;

void show(byte pattern) {
  digitalWrite(latchPin, LOW);
  shiftOut(dataPin, clockPin, MSBFIRST, pattern);
  digitalWrite(latchPin, HIGH);
}

void setup() {
  pinMode(dataPin, OUTPUT);
  pinMode(clockPin, OUTPUT);
  pinMode(latchPin, OUTPUT);
}

void loop() {
  for (int i = 0; i < 8; i++) {
    show(1 << i);
    delay(100);
  }
  for (int i = 6; i >= 1; i--) {
    show(1 << i);
    delay(100);
  }
}
