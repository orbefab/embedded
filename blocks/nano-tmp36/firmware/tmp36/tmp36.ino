const int sensorPin = A0;
// On USB, the Nano's 5V rail (AVCC) sits near 4.76 V after the MBR0520 Schottky diode.
const float vRef = 4.76f;

int lastAdc = -1;

void sampleAndReport() {
  int adc = analogRead(sensorPin);
  if (adc == lastAdc) {
    return;
  }
  lastAdc = adc;

  float voltage = ((float)adc / 1024.0f) * vRef;
  float tempC = (voltage - 0.5f) * 100.0f;

  Serial.print("voltage=");
  Serial.print(voltage, 2);
  Serial.print(" V, temp=");
  Serial.print(tempC, 1);
  Serial.println(" C");
}

void setup() {
  Serial.begin(115200);
  sampleAndReport();
}

void loop() {
  sampleAndReport();
  delay(20);
}
