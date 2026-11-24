const int ledPin = 18;
int brightness = 0;
int fadeAmount = 5;

void setup() {
  Serial.begin(115200);
  ledcAttach(ledPin, 5000, 8);
  Serial.println("ready");
}

void loop() {
  ledcWrite(ledPin, brightness);
  brightness = brightness + fadeAmount;
  if (brightness <= 0 || brightness >= 255) {
    fadeAmount = -fadeAmount;
  }
  delay(30);
}
