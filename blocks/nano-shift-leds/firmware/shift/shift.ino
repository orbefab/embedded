// QA is the least significant bit. 0x55 lights QA, QC, QE and QG.
const int dataPin = 11;
const int clockPin = 12;
const int latchPin = 8;

void setup() {
  pinMode(dataPin, OUTPUT);
  pinMode(clockPin, OUTPUT);
  pinMode(latchPin, OUTPUT);
  digitalWrite(latchPin, LOW);
  shiftOut(dataPin, clockPin, MSBFIRST, 0x55);
  digitalWrite(latchPin, HIGH);
}

void loop() {
}
