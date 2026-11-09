// Pins 9 to 13 go high in setup and stay high, so a gate on pin 9 holds a MOSFET on.
// Pin 7 goes high at 100 ms and pin 8 toggles every 200 ms, edges that leave the gate alone.
void setup() {
  for (int p = 9; p <= 13; p++) {
    pinMode(p, OUTPUT);
    digitalWrite(p, HIGH);
  }
  pinMode(8, OUTPUT);
  pinMode(7, OUTPUT);
}

void loop() {
  digitalWrite(8, (millis() / 200) % 2);
  digitalWrite(7, millis() >= 100);
}
