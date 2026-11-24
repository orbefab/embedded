volatile int fades = 0;

void IRAM_ATTR onFadeDone() {
  fades = fades + 1;
}

void setup() {
  Serial.begin(115200);
  ledcAttach(4, 5000, 10);
  ledcFadeWithInterrupt(4, 0, 200, 400, onFadeDone);
  Serial.println("fading");
  delay(50);
  ledcDetach(4);
  pinMode(4, OUTPUT);
  digitalWrite(4, HIGH);
  Serial.println("gpio high");
  delay(20);
  digitalWrite(4, LOW);
  Serial.println("gpio low");
  delay(20);
  Serial.printf("after_detach %d\n", fades);
  ledcAttach(4, 2000, 8);
  ledcWrite(4, 64);
  Serial.println("reattached");
  delay(400);
  Serial.printf("after_wait %d\n", fades);
}

void loop() {
  delay(1000);
}
