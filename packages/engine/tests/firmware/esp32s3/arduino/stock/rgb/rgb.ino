void setup() {
  Serial.begin(115200);
  rgbLedWrite(38, 0, 40, 0);
  Serial.println("ready");
}

void loop() {
  delay(1000);
}
