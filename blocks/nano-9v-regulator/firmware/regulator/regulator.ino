const int batPin = A0;

int lastCentiV = -1;

void report() {
  int adc = analogRead(batPin);
  // 10k / 10k divider with 5.00 V regulated AVCC -> full scale is 10.00 V.
  float volts = (float)adc * (10.0f / 1023.0f);
  int centiV = (int)(volts * 100.0f + 0.5f);
  if (lastCentiV < 0 || abs(centiV - lastCentiV) >= 5) {
    lastCentiV = centiV;
    Serial.print("bat=");
    Serial.print(volts, 2);
    Serial.println("V");
  }
}

void setup() {
  Serial.begin(115200);
  report();
}

void loop() {
  report();
  delay(20);
}
