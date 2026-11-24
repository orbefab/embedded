void setup() {
  Serial.begin(115200);
  Serial.println("burst");
  Serial.flush();
  delay(2);
  delayMicroseconds(500);
  tone(4, 4000, 1);
  tone(4, 8000, 1);
  tone(4, 4321, 1);
  delay(8);
  Serial.println("spaced");
  Serial.flush();
  delay(2);
  delayMicroseconds(600);
  tone(4, 4000, 1);
  delay(2);
  tone(4, 8000, 1);
  delay(2);
  tone(4, 4321, 1);
  delay(5);
  Serial.println("done");
}

void loop() {
  delay(1000);
}
