// Segments a to g on D2 to D8 show 8. The common cathode on D10 is low 55 of each 255 PWM steps.
void setup() {
  for (int pin = 2; pin <= 8; pin++) {
    pinMode(pin, OUTPUT);
    digitalWrite(pin, HIGH);
  }
  analogWrite(10, 200);
}

void loop() {}
