// Reads A6 and A7 every 10 ms and prints both codes, as "A6=682 A7=341". A6 and A7 are the
// ATmega328P TQFP's ADC6 and ADC7, analog inputs with no digital pin behind them.

void setup() {
  Serial.begin(115200);
}

void loop() {
  int a6 = analogRead(A6);
  int a7 = analogRead(A7);
  Serial.print("A6=");
  Serial.print(a6);
  Serial.print(" A7=");
  Serial.println(a7);
  delay(10);
}
