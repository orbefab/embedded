void setup() {
  Serial.begin(115200);
  ledcAttach(4, 1000, 10);
  ledcAttach(5, 1000, 10);
  ledcWrite(4, 256);
  ledcWrite(5, 768);
  Serial.println("first");
  delay(20);
  ledcChangeFrequency(4, 2000, 10);
  Serial.println("second");
  delay(20);
  Serial.println("done");
}

void loop() {
  delay(1000);
}
