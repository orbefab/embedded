void setup() {
  Serial.begin(115200);
  tone(4, 1000);
  Serial.println("tone");
  delay(50);
  noTone(4);
  Serial.println("done");
}

void loop() {
  delay(1000);
}
