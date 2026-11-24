const int pin = 8;

void setup() {
}

void loop() {
  tone(pin, 440, 200);
  delay(400);
  tone(pin, 880, 200);
  delay(400);
}
