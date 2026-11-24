volatile int fades = 0;

void IRAM_ATTR onFadeDone() {
  fades = fades + 1;
}

void setup() {
  Serial.begin(115200);
  ledcAttach(4, 5000, 10);
  ledcFade(4, 0, 1023, 100);
  Serial.println("fade1");
  delay(150);
  Serial.printf("duty1 %lu\n", (unsigned long)ledcRead(4));
  ledcFadeWithInterrupt(4, 1023, 0, 100, onFadeDone);
  Serial.println("fade2");
  delay(150);
  Serial.printf("done %d duty2 %lu\n", fades, (unsigned long)ledcRead(4));
}

void loop() {
  delay(1000);
}
