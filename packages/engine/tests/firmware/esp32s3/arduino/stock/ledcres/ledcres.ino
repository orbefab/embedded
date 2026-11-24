void setup() {
  Serial.begin(115200);
  ledcAttach(4, 1000, 10);
  ledcWrite(4, 256);
  Serial.println("p1");
  delay(20);
  ledcChangeFrequency(4, 2000, 8);
  Serial.printf("p2 %lu\n", (unsigned long)ledcRead(4));
  delay(20);
  ledcChangeFrequency(4, 1000, 10);
  ledcWrite(4, 200);
  Serial.println("p3");
  delay(20);
  ledcChangeFrequency(4, 2000, 8);
  Serial.println("p4");
  delay(20);
  ledcWrite(4, 64);
  delay(5);
  ledcChangeFrequency(4, 400, 13);
  Serial.println("p5");
  delay(20);
  Serial.println("done");
}

void loop() {
  delay(1000);
}
