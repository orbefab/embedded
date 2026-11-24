const int servoPin = 18;
const uint16_t widths[] = {1000, 1500, 2000};

void setup() {
  Serial.begin(115200);
  ledcAttach(servoPin, 50, 14);
  Serial.println("ready");
}

void loop() {
  for (uint16_t us : widths) {
    ledcWrite(servoPin, us * 16384UL / 20000);
    Serial.printf("pulse %u\n", us);
    delay(200);
  }
}
