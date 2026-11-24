void setup() {
  Serial.begin(115200);
  tone(4, 1000);
  Serial.println("tone1");
  delay(30);
  noTone(4);
  Serial.println("quiet");
  delay(20);
  tone(4, 2500);
  Serial.println("tone2");
  delay(30);
}

void loop() {
  delay(1000);
}
