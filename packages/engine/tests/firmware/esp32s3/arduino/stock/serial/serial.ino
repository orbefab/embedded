void setup() {
  Serial.begin(115200);
  Serial.println("ready");
}

void loop() {
  Serial.println("tick");
  delay(100);
}
