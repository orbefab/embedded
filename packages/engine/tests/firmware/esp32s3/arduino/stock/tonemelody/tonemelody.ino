const unsigned int notes[] = {262, 294, 330, 349, 392};

void setup() {
  Serial.begin(115200);
  for (int i = 0; i < 5; i++) {
    tone(4, notes[i], 200);
    Serial.printf("note %d %u\n", i, notes[i]);
    delay(210);
  }
  Serial.println("done");
}

void loop() {
  delay(1000);
}
