void setup() {
  Serial.begin(115200);
  ledcAttach(4, 1000, 10);
  ledcWriteTone(4, 440);
  Serial.println("tone440");
  delay(20);
  ledcWriteTone(4, 880);
  Serial.println("tone880");
  delay(20);
  Serial.println("done");
}

void loop() {
  delay(1000);
}
