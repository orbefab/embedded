void setup() {
  Serial.begin(115200);
  ledcAttach(4, 5000, 10);
  ledcWrite(4, 512);
  Serial.println("first");
  delay(10);
  ledcDetach(4);
  Serial.println("detached");
  delay(10);
  ledcAttach(4, 2000, 8);
  ledcWrite(4, 64);
  Serial.println("second");
}

void loop() {
  delay(1000);
}
