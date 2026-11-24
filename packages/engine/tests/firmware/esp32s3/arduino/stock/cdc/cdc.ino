const int LED_PIN = 4;

void setup() {
  pinMode(LED_PIN, OUTPUT);
  Serial.begin(115200);
  Serial.println("cdc-ready");
  digitalWrite(LED_PIN, HIGH);
}

void loop() {
  int c = Serial.read();
  if (c >= 0) {
    Serial.printf("got %d\n", c);
  }
  delay(20);
}
