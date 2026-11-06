void setup() {
  Serial.begin(115200);
  for (int i = 0; i < 40; i++) Serial.print("\xc3\xa9");
  Serial.print("|bad\xff\xfe|");
  Serial.print("\xe2\x82");
  delay(20);
  Serial.print("\xac done");
}
void loop() {}
