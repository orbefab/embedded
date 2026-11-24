#include <SPI.h>

void setup() {
  Serial.begin(115200);

  int a1 = analogRead(1);
  int a2 = analogRead(2);
  int a14 = analogRead(14);

  SPI.begin();
  uint8_t got = SPI.transfer(0x55);

  const int outs[] = {3, 4, 5, 6, 7, 8, 9, 10, 15, 16, 17, 18, 21};
  for (int p : outs) {
    pinMode(p, OUTPUT);
    digitalWrite(p, HIGH);
  }
  pinMode(0, INPUT);

  Serial.printf("adc %d %d %d spi %02x\n", a1, a2, a14, got);
  Serial.println("ready");
}

void loop() {
  delay(100);
}
