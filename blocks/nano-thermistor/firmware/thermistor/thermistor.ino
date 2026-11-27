#include <math.h>

const int sensorPin = A0;
const int ledPin = 9;
const float rFixed = 10000.0f;
const float r25 = 10000.0f;
const float beta = 3977.0f;
const float t25K = 298.15f;

int lastTenths = -10000;

void sampleAndReport() {
  int adc = analogRead(sensorPin);
  if (adc <= 0 || adc >= 1023) {
    return;
  }
  float rNtc = rFixed * (1023.0f / (float)adc - 1.0f);
  float tempK = 1.0f / (1.0f / t25K + logf(rNtc / r25) / beta);
  float tempC = tempK - 273.15f;
  digitalWrite(ledPin, tempC >= 30.0f ? HIGH : LOW);

  int tenths = (int)roundf(tempC * 10.0f);
  if (tenths != lastTenths) {
    lastTenths = tenths;
    Serial.print("temp=");
    Serial.print(tempC, 1);
    Serial.println(" C");
  }
}

void setup() {
  pinMode(ledPin, OUTPUT);
  Serial.begin(115200);
  sampleAndReport();
}

void loop() {
  sampleAndReport();
  delay(20);
}
