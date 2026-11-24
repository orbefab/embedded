void setup() {
  Serial.begin(115200);
  analogWrite(18, 128);
  Serial.println("ready");
}

void loop() {
  delay(1000);
}
