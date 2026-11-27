#include <Servo.h>

Servo servo;
const int sensePin = A0;

void printPack() {
  int adc = analogRead(sensePin);
  // 10k / 10k divider with ~4.76 V USB AVCC.
  float volts = (float)adc * (4.76f / 1023.0f) * 2.0f;
  Serial.print("pack=");
  Serial.print(volts, 2);
  Serial.println("V");
}

void setup() {
  Serial.begin(115200);
  // Hold at the 90 deg centre first so the idle pack voltage is easy to read.
  servo.write(90);
  servo.attach(9);
  delay(80);
  printPack();
}

void loop() {
  servo.write(0);
  delay(100);
  printPack();
  delay(350);
  servo.write(180);
  delay(100);
  printPack();
  delay(500);
}
