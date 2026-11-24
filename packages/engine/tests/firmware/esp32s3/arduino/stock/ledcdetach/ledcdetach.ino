void setup() {
  Serial.begin(115200);
  ledcAttach(4, 5000, 10);
  ledcWrite(4, 512);
  Serial.println("attached");
  delay(10);
  ledcDetach(4);
  pinMode(4, OUTPUT);
  digitalWrite(4, HIGH);
  Serial.println("gpio high");
  delay(10);
  digitalWrite(4, LOW);
  Serial.println("gpio low");
}

void loop() {
  delay(1000);
}
