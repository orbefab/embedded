// Stock ESP32-S3 sketch: no delay(), loop() polls millis() and toggles GPIO4 every 500 ms.
const int ledPin = 4;
unsigned long last = 0;
bool on = false;

void setup() {
  pinMode(ledPin, OUTPUT);
}

void loop() {
  unsigned long now = millis();
  if (now - last >= 500) {
    last += 500;
    on = !on;
    digitalWrite(ledPin, on ? HIGH : LOW);
  }
}
