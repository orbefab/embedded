void setup() {
  Serial.begin(115200);
  Serial.println("ready");
}

void loop() {
  Serial.print("raw ");
  Serial.print(analogRead(6));
  Serial.print(" mv ");
  Serial.println(analogReadMilliVolts(6));
  delay(20);
}
