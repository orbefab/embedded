void setup() {
  Serial.begin(115200);
  Serial.println("ready");
}

void loop() {
  Serial.print("temp ");
  Serial.println(temperatureRead(), 1);
  delay(50);
}
