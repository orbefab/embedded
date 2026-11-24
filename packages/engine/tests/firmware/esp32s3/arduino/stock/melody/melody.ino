const unsigned int notes[] = {262, 330, 392, 523};

void setup() {
  Serial.begin(115200);
  for (int i = 0; i < 4; i++) {
    tone(4, notes[i]);
    Serial.printf("note %d %u\n", i, notes[i]);
    delay(25);
    noTone(4);
    Serial.printf("rest %d\n", i);
    delay(10);
  }
  Serial.println("done");
}

void loop() {
  delay(1000);
}
